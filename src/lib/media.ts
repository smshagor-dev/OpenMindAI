import { invoke } from "@tauri-apps/api/core";
import {
  ALL_FORMATS,
  AudioBufferSink,
  BlobSource,
  CanvasSink,
  Input,
} from "mediabunny";
import type { Artifact } from "../types";

export interface TranscriptionResult {
  text: string;
  language: string | null;
  durationSeconds: number;
}

export interface VisionMediaDraft {
  kind: "image";
  name: string;
  mimeType: "image/jpeg" | "image/png";
  dataUrl: string;
}

export interface VideoAnalysisResult {
  durationSeconds: number;
  transcript: TranscriptionResult | null;
  frames: VisionMediaDraft[];
}

const TARGET_SAMPLE_RATE = 16_000;
const MAX_AUDIO_DURATION_SECONDS = 60 * 60;
const MAX_VIDEO_DURATION_SECONDS = 2 * 60 * 60;
const MAX_AUDIO_FILE_BYTES = 256 * 1024 * 1024;
const MAX_VIDEO_FILE_BYTES = 1024 * 1024 * 1024;
const MAX_VIDEO_FRAMES = 4;
const VIDEO_FRAME_MAX_DIMENSION = 1280;
const TRANSCRIPTION_CHUNK_SECONDS = 5 * 60;
const TRANSCRIPTION_CHUNK_SAMPLES = TARGET_SAMPLE_RATE * TRANSCRIPTION_CHUNK_SECONDS;

export async function transcribeAudioBlob(blob: Blob, sourceName = "microphone.wav") {
  const audioBuffer = await decodeAudioWithWebAudio(blob);
  return transcribeAudioBuffer(audioBuffer, sourceName);
}

export async function transcribeAudioFile(file: File) {
  if (file.size === 0) throw new Error("The audio file is empty.");
  if (file.size > MAX_AUDIO_FILE_BYTES) {
    throw new Error("Audio file is too large. Maximum supported size is 256 MB.");
  }

  const input = new Input({ formats: ALL_FORMATS, source: new BlobSource(file) });
  try {
    if (!(await input.canRead())) {
      throw new Error("This audio container is not recognized by the local media reader.");
    }
    const track = await input.getPrimaryAudioTrack();
    if (!track) throw new Error("No audio track was found in this file.");
    if (!(await track.canDecode())) {
      throw new Error("The audio codec in this file is not supported by the system decoder.");
    }
    const start = Math.max(0, await track.getFirstTimestamp());
    const end = await track.computeDuration();
    const duration = Math.max(0, end - start);
    if (duration <= 0) throw new Error("The audio contains no decodable samples.");
    if (duration > MAX_AUDIO_DURATION_SECONDS) {
      throw new Error("Audio is too long. Maximum supported duration is 60 minutes.");
    }
    const sink = new AudioBufferSink(track);
    return await transcribeWrappedAudio(sink.buffers(start, end), file.name);
  } catch (error) {
    // Web Audio remains a useful fallback for formats handled natively by the
    // WebView but not by its WebCodecs implementation.
    try {
      const audio = await decodeAudioWithWebAudio(file);
      return await transcribeAudioBuffer(audio, file.name);
    } catch {
      throw error;
    }
  } finally {
    input.dispose();
  }
}

export async function analyzeVideoFile(file: File): Promise<VideoAnalysisResult> {
  if (file.size === 0) throw new Error("The video file is empty.");
  if (file.size > MAX_VIDEO_FILE_BYTES) {
    throw new Error("Video file is too large. Maximum supported size is 1 GB.");
  }

  const input = new Input({ formats: ALL_FORMATS, source: new BlobSource(file) });
  try {
    if (!(await input.canRead())) {
      throw new Error("This video container is not recognized by the local media reader.");
    }
    const duration = await input.computeDuration();
    if (!Number.isFinite(duration) || duration <= 0) {
      throw new Error("Could not determine the video duration.");
    }
    if (duration > MAX_VIDEO_DURATION_SECONDS) {
      throw new Error("Video is too long. Maximum supported duration is 2 hours.");
    }

    const videoTrack = await input.getPrimaryVideoTrack();
    if (!videoTrack) throw new Error("No video track was found in this file.");
    if (!(await videoTrack.canDecode())) {
      throw new Error("The video codec is not supported by the system decoder.");
    }
    const frames = await sampleVideoFrames(videoTrack, file.name);

    let transcript: TranscriptionResult | null = null;
    const audioTrack = await input.getPrimaryAudioTrack();
    if (audioTrack && (await audioTrack.canDecode())) {
      const start = Math.max(0, await audioTrack.getFirstTimestamp());
      const end = await audioTrack.computeDuration();
      if (Math.max(0, end - start) <= MAX_AUDIO_DURATION_SECONDS) {
        const sink = new AudioBufferSink(audioTrack);
        try {
          transcript = await transcribeWrappedAudio(sink.buffers(start, end), file.name);
        } catch {
          transcript = null;
        }
      }
    }

    return { durationSeconds: duration, transcript, frames };
  } finally {
    input.dispose();
  }
}

export function createSoundscapeArtifact(
  conversationId: string,
  messageId: string | null,
  prompt: string,
) {
  return invoke<Artifact>("create_soundscape_artifact", {
    conversationId,
    messageId,
    prompt,
  });
}

export function artifactMediaDataUrl(artifactId: string) {
  return invoke<string>("artifact_media_data_url", { artifactId });
}

async function transcribeAudioBuffer(audioBuffer: AudioBuffer, sourceName: string) {
  if (!Number.isFinite(audioBuffer.duration) || audioBuffer.duration <= 0) {
    throw new Error("The audio contains no decodable samples.");
  }
  if (audioBuffer.duration > MAX_AUDIO_DURATION_SECONDS) {
    throw new Error("Audio is too long. Maximum supported duration is 60 minutes.");
  }

  async function* oneBuffer() {
    yield { buffer: audioBuffer, duration: audioBuffer.duration };
  }
  return transcribeWrappedAudio(oneBuffer(), sourceName);
}

async function transcribeWrappedAudio(
  buffers: AsyncIterable<{ buffer: AudioBuffer; duration: number }>,
  sourceName: string,
): Promise<TranscriptionResult> {
  const pending: Float32Array[] = [];
  let pendingSamples = 0;
  let totalSeconds = 0;
  let language: string | null = null;
  const transcriptParts: string[] = [];

  const flush = async () => {
    if (pendingSamples === 0) return;
    const samples = concatenateFloat32(pending, pendingSamples);
    pending.length = 0;
    pendingSamples = 0;
    const result = await transcribeSamples(samples, sourceName);
    if (!language && result.language) language = result.language;
    if (result.text.trim()) transcriptParts.push(result.text.trim());
  };

  for await (const item of buffers) {
    if (!item.buffer || item.buffer.length === 0) continue;
    totalSeconds += Math.max(0, item.duration || item.buffer.duration);
    if (totalSeconds > MAX_AUDIO_DURATION_SECONDS + 1) {
      throw new Error("Audio is too long. Maximum supported duration is 60 minutes.");
    }
    const mono = mixToMono(item.buffer);
    const resampled = resampleLinear(mono, item.buffer.sampleRate, TARGET_SAMPLE_RATE);
    if (pendingSamples > 0 && pendingSamples + resampled.length > TRANSCRIPTION_CHUNK_SAMPLES) {
      await flush();
    }
    if (resampled.length > TRANSCRIPTION_CHUNK_SAMPLES) {
      for (let offset = 0; offset < resampled.length; offset += TRANSCRIPTION_CHUNK_SAMPLES) {
        pending.push(resampled.slice(offset, offset + TRANSCRIPTION_CHUNK_SAMPLES));
        pendingSamples += pending[pending.length - 1].length;
        await flush();
      }
    } else {
      pending.push(resampled);
      pendingSamples += resampled.length;
    }
  }
  await flush();

  const text = transcriptParts.join(" ").replace(/\s+/g, " ").trim();
  if (!text) throw new Error("OpenMindAI Hear did not detect speech in this audio.");
  return { text, language, durationSeconds: totalSeconds };
}

async function transcribeSamples(samples: Float32Array, sourceName: string) {
  const wav = encodePcm16Wav(samples, TARGET_SAMPLE_RATE);
  const dataUrl = await blobToDataUrl(new Blob([wav], { type: "audio/wav" }));
  return invoke<TranscriptionResult>("transcribe_audio", {
    audioDataUrl: dataUrl,
    sourceName,
  });
}

async function decodeAudioWithWebAudio(blob: Blob) {
  const AudioContextCtor = window.AudioContext;
  if (!AudioContextCtor) {
    throw new Error("This system does not expose the browser audio decoder required for local transcription.");
  }
  const context = new AudioContextCtor();
  try {
    const bytes = await blob.arrayBuffer();
    return await context.decodeAudioData(bytes.slice(0));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`Could not decode this audio format locally: ${detail}`);
  } finally {
    await context.close().catch(() => undefined);
  }
}

function mixToMono(audio: AudioBuffer) {
  const mono = new Float32Array(audio.length);
  for (let channel = 0; channel < audio.numberOfChannels; channel += 1) {
    const values = audio.getChannelData(channel);
    for (let index = 0; index < audio.length; index += 1) {
      mono[index] += values[index] / audio.numberOfChannels;
    }
  }
  return mono;
}

function resampleLinear(input: Float32Array, sourceRate: number, targetRate: number) {
  if (sourceRate === targetRate) return input;
  const duration = input.length / sourceRate;
  const length = Math.max(1, Math.round(duration * targetRate));
  const output = new Float32Array(length);
  const ratio = sourceRate / targetRate;
  for (let index = 0; index < length; index += 1) {
    const position = index * ratio;
    const left = Math.min(input.length - 1, Math.floor(position));
    const right = Math.min(input.length - 1, left + 1);
    const fraction = position - left;
    output[index] = input[left] * (1 - fraction) + input[right] * fraction;
  }
  return output;
}

function concatenateFloat32(chunks: Float32Array[], totalLength: number) {
  const output = new Float32Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
}

function encodePcm16Wav(samples: Float32Array, sampleRate: number) {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, samples.length * 2, true);
  let offset = 44;
  for (const sample of samples) {
    const value = Math.max(-1, Math.min(1, sample));
    view.setInt16(offset, Math.round(value < 0 ? value * 0x8000 : value * 0x7fff), true);
    offset += 2;
  }
  return buffer;
}

function writeAscii(view: DataView, offset: number, value: string) {
  for (let index = 0; index < value.length; index += 1) {
    view.setUint8(offset + index, value.charCodeAt(index));
  }
}

function blobToDataUrl(blob: Blob) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error("Could not encode local audio."));
    reader.readAsDataURL(blob);
  });
}

async function sampleVideoFrames(
  videoTrack: Awaited<ReturnType<Input["getPrimaryVideoTrack"]>> & {},
  sourceName: string,
) {
  const start = await videoTrack.getFirstTimestamp();
  const end = await videoTrack.computeDuration();
  const duration = Math.max(0.001, end - start);
  const fractions = duration < 3 ? [0.25, 0.7] : [0.08, 0.34, 0.64, 0.92];
  const timestamps = fractions
    .slice(0, MAX_VIDEO_FRAMES)
    .map((fraction) => Math.max(start, Math.min(end - 0.001, start + duration * fraction)));

  const displayWidth = Math.max(1, await videoTrack.getDisplayWidth());
  const displayHeight = Math.max(1, await videoTrack.getDisplayHeight());
  const scale = Math.min(1, VIDEO_FRAME_MAX_DIMENSION / Math.max(displayWidth, displayHeight));
  const sink = new CanvasSink(videoTrack, {
    width: Math.max(1, Math.round(displayWidth * scale)),
    height: Math.max(1, Math.round(displayHeight * scale)),
    fit: "contain",
    alpha: false,
  });

  const frames: VisionMediaDraft[] = [];
  for await (const result of sink.canvasesAtTimestamps(timestamps)) {
    if (!result) continue;
    const dataUrl = await canvasToJpegDataUrl(result.canvas);
    frames.push({
      kind: "image",
      name: `${sourceName} @ ${formatTimestamp(result.timestamp)}`,
      mimeType: "image/jpeg",
      dataUrl,
    });
  }
  if (frames.length === 0) {
    throw new Error("Could not decode representative frames from this video.");
  }
  return frames;
}

async function canvasToJpegDataUrl(canvas: HTMLCanvasElement | OffscreenCanvas) {
  if ("toDataURL" in canvas) {
    return canvas.toDataURL("image/jpeg", 0.82);
  }
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.82 });
  return blobToDataUrl(blob);
}

function formatTimestamp(seconds: number) {
  const whole = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

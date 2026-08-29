import { invoke } from "@tauri-apps/api/core";
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

export async function transcribeAudioBlob(blob: Blob, sourceName = "microphone.wav") {
  const audioBuffer = await decodeAudio(blob);
  return transcribeAudioBuffer(audioBuffer, sourceName);
}

export async function transcribeAudioFile(file: File) {
  if (file.size === 0) throw new Error("The audio file is empty.");
  if (file.size > MAX_AUDIO_FILE_BYTES) {
    throw new Error("Audio file is too large. Maximum supported size is 256 MB.");
  }
  return transcribeAudioBlob(file, file.name);
}

export async function analyzeVideoFile(file: File): Promise<VideoAnalysisResult> {
  if (file.size === 0) throw new Error("The video file is empty.");
  if (file.size > MAX_VIDEO_FILE_BYTES) {
    throw new Error("Video file is too large. Maximum supported size is 1 GB.");
  }

  const url = URL.createObjectURL(file);
  try {
    const video = document.createElement("video");
    video.preload = "metadata";
    video.muted = true;
    video.playsInline = true;
    video.src = url;
    await waitForVideoMetadata(video);

    if (!Number.isFinite(video.duration) || video.duration <= 0) {
      throw new Error("Could not determine the video duration.");
    }
    if (video.duration > MAX_VIDEO_DURATION_SECONDS) {
      throw new Error("Video is too long. Maximum supported duration is 2 hours.");
    }

    const frames = await sampleVideoFrames(video, file.name);
    let transcript: TranscriptionResult | null = null;
    try {
      const audio = await decodeAudio(file);
      transcript = await transcribeAudioBuffer(audio, file.name);
    } catch {
      // Some WebView codecs expose video playback but not demuxed audio to
      // AudioContext. Visual analysis still works from sampled keyframes.
    }

    return {
      durationSeconds: video.duration,
      transcript,
      frames,
    };
  } finally {
    URL.revokeObjectURL(url);
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

  const mono = mixToMono(audioBuffer);
  const resampled = resampleLinear(mono, audioBuffer.sampleRate, TARGET_SAMPLE_RATE);
  const wav = encodePcm16Wav(resampled, TARGET_SAMPLE_RATE);
  const dataUrl = await blobToDataUrl(new Blob([wav], { type: "audio/wav" }));
  return invoke<TranscriptionResult>("transcribe_audio", {
    audioDataUrl: dataUrl,
    sourceName,
  });
}

async function decodeAudio(blob: Blob) {
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

function waitForVideoMetadata(video: HTMLVideoElement) {
  return new Promise<void>((resolve, reject) => {
    if (video.readyState >= HTMLMediaElement.HAVE_METADATA) {
      resolve();
      return;
    }
    const cleanup = () => {
      video.onloadedmetadata = null;
      video.onerror = null;
    };
    video.onloadedmetadata = () => {
      cleanup();
      resolve();
    };
    video.onerror = () => {
      cleanup();
      reject(new Error("Could not decode the selected video."));
    };
  });
}

async function sampleVideoFrames(video: HTMLVideoElement, sourceName: string) {
  const fractions = video.duration < 3 ? [0.25, 0.7] : [0.08, 0.34, 0.64, 0.92];
  const frames: VisionMediaDraft[] = [];
  for (let index = 0; index < Math.min(MAX_VIDEO_FRAMES, fractions.length); index += 1) {
    const timestamp = Math.max(0, Math.min(video.duration - 0.02, video.duration * fractions[index]));
    await seekVideo(video, timestamp);
    const width = Math.max(1, video.videoWidth);
    const height = Math.max(1, video.videoHeight);
    const scale = Math.min(1, VIDEO_FRAME_MAX_DIMENSION / Math.max(width, height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare a video frame for local vision.");
    context.drawImage(video, 0, 0, canvas.width, canvas.height);
    frames.push({
      kind: "image",
      name: `${sourceName} @ ${formatTimestamp(timestamp)}`,
      mimeType: "image/jpeg",
      dataUrl: canvas.toDataURL("image/jpeg", 0.82),
    });
  }
  return frames;
}

function seekVideo(video: HTMLVideoElement, timestamp: number) {
  return new Promise<void>((resolve, reject) => {
    if (Math.abs(video.currentTime - timestamp) < 0.02 && video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
      resolve();
      return;
    }
    const cleanup = () => {
      video.onseeked = null;
      video.onerror = null;
    };
    video.onseeked = () => {
      cleanup();
      resolve();
    };
    video.onerror = () => {
      cleanup();
      reject(new Error("Could not seek the selected video."));
    };
    video.currentTime = timestamp;
  });
}

function formatTimestamp(seconds: number) {
  const whole = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

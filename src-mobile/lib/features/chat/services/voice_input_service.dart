import 'dart:async';

import 'package:record/record.dart';
import 'package:whisper_ggml/whisper_ggml.dart';

class VoiceTranscriptEvent {
  const VoiceTranscriptEvent({
    required this.text,
    required this.isFinal,
  });

  final String text;
  final bool isFinal;
}

class VoiceInputService {
  VoiceInputService({
    AudioRecorder? recorder,
    WhisperController? whisper,
  })  : _recorder = recorder ?? AudioRecorder(),
        _whisper = whisper ?? WhisperController();

  final AudioRecorder _recorder;
  final WhisperController _whisper;
  final StreamController<VoiceTranscriptEvent> _events =
      StreamController<VoiceTranscriptEvent>.broadcast();

  WhisperLiveSession? _session;
  StreamSubscription<String>? _partialsSubscription;
  bool _listening = false;
  bool _disposed = false;

  Stream<VoiceTranscriptEvent> get events => _events.stream;
  bool get isListening => _listening;

  Future<void> start() async {
    if (_disposed) {
      throw const VoiceInputException('Voice input is no longer available.');
    }
    if (_listening) return;

    final granted = await _recorder.hasPermission();
    if (!granted) {
      throw const VoiceInputException(
        'Microphone permission is required for OpenMindAI Hear.',
      );
    }

    _listening = true;
    try {
      final pcmStream = await _recorder.startStream(
        const RecordConfig(
          encoder: AudioEncoder.pcm16bits,
          sampleRate: 16000,
          numChannels: 1,
          autoGain: true,
          echoCancel: true,
          noiseSuppress: true,
        ),
      );

      final session = await _whisper.transcribeLive(
        model: WhisperModel.base,
        pcm16Stream: pcmStream,
        lang: 'auto',
        keepModelLoaded: true,
      );
      _session = session;
      _partialsSubscription = session.partials.listen(
        (text) {
          final normalized = text.trim();
          if (normalized.isEmpty || _events.isClosed) return;
          _events.add(VoiceTranscriptEvent(text: normalized, isFinal: false));
        },
        onError: (Object error, StackTrace stackTrace) {
          if (!_events.isClosed) _events.addError(error, stackTrace);
        },
      );
    } catch (error) {
      _listening = false;
      await _safeStopRecorder();
      rethrow;
    }
  }

  Future<String> stop() async {
    if (!_listening) return '';
    _listening = false;

    final session = _session;
    _session = null;
    await _partialsSubscription?.cancel();
    _partialsSubscription = null;
    await _safeStopRecorder();

    if (session == null) return '';
    final text = (await session.stop()).trim();
    if (text.isNotEmpty && !_events.isClosed) {
      _events.add(VoiceTranscriptEvent(text: text, isFinal: true));
    }
    return text;
  }

  Future<void> cancel() async {
    if (!_listening && _session == null) return;
    _listening = false;
    final session = _session;
    _session = null;
    await _partialsSubscription?.cancel();
    _partialsSubscription = null;
    await _safeStopRecorder();
    if (session != null) {
      await session.stop();
    }
  }

  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    await cancel();
    await _whisper.releaseModel();
    await _recorder.dispose();
    await _events.close();
  }

  Future<void> _safeStopRecorder() async {
    try {
      await _recorder.stop();
    } catch (_) {
      // Recorder can already be stopped by an OS interruption.
    }
  }
}

class VoiceInputException implements Exception {
  const VoiceInputException(this.message);

  final String message;

  @override
  String toString() => message;
}

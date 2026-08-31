import 'package:flutter_tts/flutter_tts.dart';

class SpeechOutputService {
  final FlutterTts _tts = FlutterTts();
  bool _configured = false;

  Future<void> _configure() async {
    if (_configured) return;
    await _tts.awaitSpeakCompletion(true);
    await _tts.setLanguage('en-US');
    await _tts.setSpeechRate(0.48);
    await _tts.setVolume(1.0);
    await _tts.setPitch(1.0);
    _configured = true;
  }

  Future<void> speak(String markdownText) async {
    final text = _plainSpeechText(markdownText);
    if (text.isEmpty) return;
    await _configure();
    await _tts.stop();
    final result = await _tts.speak(text);
    if (result != 1) {
      throw const SpeechOutputException(
        'OpenMindAI Speak could not start on this device.',
      );
    }
  }

  Future<void> stop() async {
    await _tts.stop();
  }

  Future<void> dispose() async {
    await _tts.stop();
  }

  String _plainSpeechText(String value) {
    var text = value;
    text = text.replaceAll(RegExp(r'```[\s\S]*?```'), ' Code block omitted. ');
    text = text.replaceAll(RegExp(r'`([^`]+)`'), r'$1');
    text = text.replaceAll(RegExp(r'!\[[^\]]*\]\([^\)]*\)'), ' Image. ');
    text = text.replaceAll(RegExp(r'\[([^\]]+)\]\([^\)]*\)'), r'$1');
    text = text.replaceAll(RegExp(r'^\s{0,3}#{1,6}\s+', multiLine: true), '');
    text = text.replaceAll(RegExp(r'^\s*>\s?', multiLine: true), '');
    text = text.replaceAll(RegExp(r'^\s*[-*+]\s+', multiLine: true), '');
    text = text.replaceAll(RegExp(r'^\s*\d+[.)]\s+', multiLine: true), '');
    text = text.replaceAll(RegExp(r'[*_~]'), '');
    text = text.replaceAll(RegExp(r'\s+'), ' ').trim();
    return text;
  }
}

class SpeechOutputException implements Exception {
  const SpeechOutputException(this.message);

  final String message;

  @override
  String toString() => message;
}

import 'package:flutter_tts/flutter_tts.dart';

class SpeechOutputService {
  static const int _maxChunkCharacters = 3200;

  final FlutterTts _tts = FlutterTts();
  bool _configured = false;
  int _session = 0;

  Future<void> _configure() async {
    if (_configured) return;
    await _tts.awaitSpeakCompletion(true);
    await _tts.setSpeechRate(0.48);
    await _tts.setVolume(1.0);
    await _tts.setPitch(1.0);
    _configured = true;
  }

  Future<void> _configureLanguage(String text) async {
    final preferred = RegExp(r'[\u0980-\u09FF]').hasMatch(text)
        ? const ['bn-BD', 'bn-IN', 'en-US']
        : const ['en-US'];
    for (final language in preferred) {
      final available = await _tts.isLanguageAvailable(language);
      if (available == true) {
        await _tts.setLanguage(language);
        return;
      }
    }
  }

  Future<void> speak(String markdownText) async {
    final text = normalizeSpeechText(markdownText);
    if (text.isEmpty) return;
    final session = ++_session;
    await _configure();
    await _configureLanguage(text);
    await _tts.stop();

    for (final chunk in splitSpeechText(text, _maxChunkCharacters)) {
      if (session != _session) return;
      final result = await _tts.speak(chunk);
      if (session != _session) return;
      if (result != 1) {
        throw const SpeechOutputException(
          'OpenMindAI Speak could not start on this device.',
        );
      }
    }
  }

  Future<void> stop() async {
    _session += 1;
    await _tts.stop();
  }

  Future<void> dispose() async {
    await stop();
  }
}

String normalizeSpeechText(String value) {
  var text = value;
  text = text.replaceAll(RegExp(r'```[\s\S]*?```'), ' Code block omitted. ');
  text = text.replaceAllMapped(RegExp(r'`([^`]+)`'), (match) => match.group(1) ?? '');
  text = text.replaceAll(RegExp(r'!\[[^\]]*\]\([^\)]*\)'), ' Image. ');
  text = text.replaceAllMapped(
    RegExp(r'\[([^\]]+)\]\([^\)]*\)'),
    (match) => match.group(1) ?? '',
  );
  text = text.replaceAll(RegExp(r'^\s{0,3}#{1,6}\s+', multiLine: true), '');
  text = text.replaceAll(RegExp(r'^\s*>\s?', multiLine: true), '');
  text = text.replaceAll(RegExp(r'^\s*[-*+]\s+', multiLine: true), '');
  text = text.replaceAll(RegExp(r'^\s*\d+[.)]\s+', multiLine: true), '');
  text = text.replaceAll(RegExp(r'[*_~]'), '');
  text = text.replaceAll(RegExp(r'\s+'), ' ').trim();
  return text;
}

List<String> splitSpeechText(String text, [int maxCharacters = 3200]) {
  final normalized = text.trim();
  if (normalized.isEmpty) return const [];
  if (normalized.length <= maxCharacters) return [normalized];

  final chunks = <String>[];
  var remaining = normalized;
  while (remaining.length > maxCharacters) {
    var splitAt = remaining.lastIndexOf(RegExp(r'[.!?।]\s'), maxCharacters);
    if (splitAt >= maxCharacters ~/ 2) {
      splitAt += 1;
    } else {
      splitAt = remaining.lastIndexOf(' ', maxCharacters);
      if (splitAt < maxCharacters ~/ 2) splitAt = maxCharacters;
    }
    chunks.add(remaining.substring(0, splitAt).trim());
    remaining = remaining.substring(splitAt).trimLeft();
  }
  if (remaining.isNotEmpty) chunks.add(remaining);
  return chunks;
}

class SpeechOutputException implements Exception {
  const SpeechOutputException(this.message);

  final String message;

  @override
  String toString() => message;
}

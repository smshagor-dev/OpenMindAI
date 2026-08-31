import 'package:flutter_test/flutter_test.dart';
import 'package:openmindai_mobile/features/chat/services/speech_output_service.dart';

void main() {
  group('normalizeSpeechText', () {
    test('keeps readable text and removes markdown syntax', () {
      const markdown = '''
# Result

Use `flutter analyze` and read the [Flutter guide](https://flutter.dev).

- First item
- Second item
''';

      expect(
        normalizeSpeechText(markdown),
        'Result Use flutter analyze and read the Flutter guide. First item Second item',
      );
    });

    test('omits fenced code and describes images', () {
      const markdown = '''
Before.

```dart
print('do not read source syntax');
```

![chart](chart.png)
After.
''';

      expect(
        normalizeSpeechText(markdown),
        'Before. Code block omitted. Image. After.',
      );
    });
  });
}

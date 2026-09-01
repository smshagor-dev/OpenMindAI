import 'package:flutter_test/flutter_test.dart';
import 'package:openmindai_mobile/features/canvas/canvas_generation_service.dart';

void main() {
  test('extracts a valid SVG from surrounding model text', () {
    const input =
        '''Here is the image:\n<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#111"/></svg>\nDone.''';
    final svg = sanitizeGeneratedSvg(input);
    expect(svg, startsWith('<svg'));
    expect(svg, contains('<rect'));
    expect(svg, endsWith('</svg>'));
  });

  test('rejects executable SVG content', () {
    const input =
        '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><script>alert(1)</script></svg>''';
    expect(
      () => sanitizeGeneratedSvg(input),
      throwsA(isA<CanvasGenerationException>()),
    );
  });

  test('rejects external href content', () {
    const input =
        '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><a href="https://example.com"><rect width="10" height="10"/></a></svg>''';
    expect(
      () => sanitizeGeneratedSvg(input),
      throwsA(isA<CanvasGenerationException>()),
    );
  });
}

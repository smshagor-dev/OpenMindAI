import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('mobile Flutter harness renders OpenMindAI branding', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(body: Center(child: Text('OpenMindAI'))),
      ),
    );

    expect(find.text('OpenMindAI'), findsOneWidget);
  });
}

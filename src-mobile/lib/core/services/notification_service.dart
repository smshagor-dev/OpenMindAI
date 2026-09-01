import 'package:flutter_local_notifications/flutter_local_notifications.dart';

class NotificationService {
  NotificationService._();

  static final NotificationService instance = NotificationService._();

  final FlutterLocalNotificationsPlugin _plugin =
      FlutterLocalNotificationsPlugin();
  bool _initialized = false;
  int _nextId = 1000;

  Future<void> initialize() async {
    if (_initialized) return;
    const settings = InitializationSettings(
      android: AndroidInitializationSettings('openmindai_notification'),
      iOS: DarwinInitializationSettings(
        requestAlertPermission: false,
        requestBadgePermission: false,
        requestSoundPermission: false,
      ),
    );
    await _plugin.initialize(settings: settings);
    _initialized = true;
  }

  Future<void> showResponseReady({String? conversationTitle}) async {
    await initialize();
    const details = NotificationDetails(
      android: AndroidNotificationDetails(
        'openmindai_generation',
        'OpenMindAI responses',
        channelDescription: 'Notifications when a local AI response is ready.',
        importance: Importance.defaultImportance,
        priority: Priority.defaultPriority,
      ),
      iOS: DarwinNotificationDetails(),
    );
    final title = conversationTitle?.trim();
    await _plugin.show(
      id: _nextId++,
      title: 'OpenMindAI response ready',
      body: title == null || title.isEmpty
          ? 'Your local AI response has finished.'
          : '$title is ready to read.',
      notificationDetails: details,
    );
  }
}

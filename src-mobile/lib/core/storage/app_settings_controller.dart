import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

class AppSettingsController extends ChangeNotifier {
  AppSettingsController._();

  static final instance = AppSettingsController._();

  static const _themeKey = 'openmindai.theme_mode';
  static const _compactKey = 'openmindai.compact_chat';
  static const _hapticsKey = 'openmindai.haptics';
  static const _notificationsKey = 'openmindai.completion_notifications';

  ThemeMode themeMode = ThemeMode.system;
  bool compactChat = false;
  bool haptics = true;
  bool completionNotifications = true;
  bool loaded = false;

  Future<void> load() async {
    final prefs = await SharedPreferences.getInstance();
    themeMode = switch (prefs.getString(_themeKey)) {
      'light' => ThemeMode.light,
      'dark' => ThemeMode.dark,
      _ => ThemeMode.system,
    };
    compactChat = prefs.getBool(_compactKey) ?? false;
    haptics = prefs.getBool(_hapticsKey) ?? true;
    completionNotifications = prefs.getBool(_notificationsKey) ?? true;
    loaded = true;
    notifyListeners();
  }

  Future<void> setThemeMode(ThemeMode value) async {
    themeMode = value;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_themeKey, switch (value) {
      ThemeMode.light => 'light',
      ThemeMode.dark => 'dark',
      ThemeMode.system => 'system',
    });
  }

  Future<void> setCompactChat(bool value) async {
    compactChat = value;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_compactKey, value);
  }

  Future<void> setHaptics(bool value) async {
    haptics = value;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_hapticsKey, value);
  }

  Future<void> setCompletionNotifications(bool value) async {
    completionNotifications = value;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_notificationsKey, value);
  }
}

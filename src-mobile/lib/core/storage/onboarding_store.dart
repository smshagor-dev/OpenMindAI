import 'package:shared_preferences/shared_preferences.dart';

class OnboardingStore {
  static const _completeKey = 'mobile_onboarding_complete_v1';
  static const _licenseKey = 'mobile_license_accepted_v1';
  static const _modelKey = 'mobile_selected_model_id_v1';

  final SharedPreferencesAsync _prefs = SharedPreferencesAsync();

  Future<bool> isComplete() async =>
      await _prefs.getBool(_completeKey) ?? false;

  Future<void> complete({required String selectedModelId}) async {
    await _prefs.setBool(_licenseKey, true);
    await _prefs.setString(_modelKey, selectedModelId);
    await _prefs.setBool(_completeKey, true);
  }

  Future<String?> selectedModelId() => _prefs.getString(_modelKey);

  Future<void> setSelectedModelId(String id) => _prefs.setString(_modelKey, id);

  Future<void> reset() async {
    await _prefs.remove(_completeKey);
    await _prefs.remove(_licenseKey);
    await _prefs.remove(_modelKey);
  }
}

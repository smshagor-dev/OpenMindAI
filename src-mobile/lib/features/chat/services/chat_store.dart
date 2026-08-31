import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/chat_models.dart';

class ChatStore {
  static const _key = 'mobile_chat_conversations_v1';
  final SharedPreferencesAsync _prefs = SharedPreferencesAsync();

  Future<List<ChatConversation>> load() async {
    final raw = await _prefs.getString(_key);
    if (raw == null || raw.isEmpty) return [];
    final decoded = jsonDecode(raw) as List<dynamic>;
    return decoded
        .map((item) => ChatConversation.fromJson(Map<String, dynamic>.from(item as Map)))
        .toList()
      ..sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
  }

  Future<void> save(List<ChatConversation> conversations) async {
    await _prefs.setString(
      _key,
      jsonEncode(conversations.map((conversation) => conversation.toJson()).toList()),
    );
  }
}

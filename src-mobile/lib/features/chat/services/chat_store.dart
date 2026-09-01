import 'dart:convert';
import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:sqflite/sqflite.dart';

import '../models/chat_models.dart';

class ChatStore {
  static const _legacyKey = 'mobile_chat_conversations_v1';
  static const _migrationKey = 'mobile_chat_sqlite_migrated_v1';

  Database? _db;

  Future<Database> _database() async {
    final existing = _db;
    if (existing != null && existing.isOpen) return existing;

    final support = await getApplicationSupportDirectory();
    final directory = Directory(p.join(support.path, 'database'));
    if (!await directory.exists()) {
      await directory.create(recursive: true);
    }
    final db = await openDatabase(
      p.join(directory.path, 'openmindai-mobile.db'),
      version: 1,
      onConfigure: (database) async {
        await database.execute('PRAGMA foreign_keys = ON');
      },
      onCreate: (database, _) async {
        await database.execute('''
          CREATE TABLE conversations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            updated_at INTEGER NOT NULL
          )
        ''');
        await database.execute('''
          CREATE TABLE messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            role TEXT NOT NULL,
            text TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            sort_index INTEGER NOT NULL,
            attachment_paths TEXT NOT NULL DEFAULT '[]',
            FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
          )
        ''');
        await database.execute(
          'CREATE INDEX idx_messages_conversation ON messages(conversation_id, sort_index)',
        );
      },
    );
    _db = db;
    return db;
  }

  Future<List<ChatConversation>> load() async {
    final db = await _database();
    await _migrateLegacy(db);
    final rows = await db.query('conversations', orderBy: 'updated_at DESC');
    final conversations = <ChatConversation>[];
    for (final row in rows) {
      final id = row['id'] as String;
      final messageRows = await db.query(
        'messages',
        where: 'conversation_id = ?',
        whereArgs: [id],
        orderBy: 'sort_index ASC',
      );
      conversations.add(
        ChatConversation(
          id: id,
          title: row['title'] as String,
          updatedAt: DateTime.fromMillisecondsSinceEpoch(
            row['updated_at'] as int,
          ),
          messages: messageRows.map(_messageFromRow).toList(),
        ),
      );
    }
    return conversations;
  }

  ChatMessage _messageFromRow(Map<String, Object?> row) {
    final rawAttachments = row['attachment_paths'] as String? ?? '[]';
    List<String> attachments;
    try {
      attachments = (jsonDecode(rawAttachments) as List<dynamic>)
          .map((value) => value.toString())
          .toList();
    } catch (_) {
      attachments = const [];
    }
    return ChatMessage(
      id: row['id'] as String,
      role: row['role'] as String,
      text: row['text'] as String,
      createdAt: DateTime.fromMillisecondsSinceEpoch(row['created_at'] as int),
      attachmentPaths: attachments,
    );
  }

  Future<void> save(List<ChatConversation> conversations) async {
    final db = await _database();
    await _migrateLegacy(db);
    await _replaceAll(db, conversations);
  }

  Future<void> _replaceAll(
    Database db,
    List<ChatConversation> conversations,
  ) async {
    await db.transaction((transaction) async {
      await transaction.delete('messages');
      await transaction.delete('conversations');
      for (final conversation in conversations) {
        await transaction.insert('conversations', {
          'id': conversation.id,
          'title': conversation.title,
          'updated_at': conversation.updatedAt.millisecondsSinceEpoch,
        });
        for (var index = 0; index < conversation.messages.length; index++) {
          final message = conversation.messages[index];
          await transaction.insert('messages', {
            'id': message.id,
            'conversation_id': conversation.id,
            'role': message.role,
            'text': message.text,
            'created_at': message.createdAt.millisecondsSinceEpoch,
            'sort_index': index,
            'attachment_paths': jsonEncode(message.attachmentPaths),
          });
        }
      }
    });
  }

  Future<void> _migrateLegacy(Database db) async {
    final prefs = await SharedPreferences.getInstance();
    if (prefs.getBool(_migrationKey) == true) return;

    final raw = prefs.getString(_legacyKey);
    if (raw != null && raw.trim().isNotEmpty) {
      try {
        final decoded = jsonDecode(raw) as List<dynamic>;
        final conversations = decoded
            .map(
              (item) => ChatConversation.fromJson(
                Map<String, dynamic>.from(item as Map),
              ),
            )
            .toList();
        await _replaceAll(db, conversations);
      } catch (_) {
        // Keep startup resilient if an old preference entry was malformed.
      }
    }
    await prefs.remove(_legacyKey);
    await prefs.setBool(_migrationKey, true);
  }

  Future<void> clear() async {
    final db = await _database();
    await db.transaction((transaction) async {
      await transaction.delete('messages');
      await transaction.delete('conversations');
    });
  }

  Future<int> sizeBytes() async {
    final db = await _database();
    final file = File(db.path);
    if (!await file.exists()) return 0;
    return await file.length();
  }

  Future<String> exportJson() async {
    final conversations = await load();
    return const JsonEncoder.withIndent('  ').convert(
      conversations.map((conversation) => conversation.toJson()).toList(),
    );
  }
}

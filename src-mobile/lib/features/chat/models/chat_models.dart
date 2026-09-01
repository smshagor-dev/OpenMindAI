class ChatMessage {
  const ChatMessage({
    required this.id,
    required this.role,
    required this.text,
    required this.createdAt,
    this.attachmentPaths = const [],
  });

  final String id;
  final String role;
  final String text;
  final DateTime createdAt;
  final List<String> attachmentPaths;

  ChatMessage copyWith({String? text}) => ChatMessage(
    id: id,
    role: role,
    text: text ?? this.text,
    createdAt: createdAt,
    attachmentPaths: attachmentPaths,
  );

  Map<String, dynamic> toJson() => {
    'id': id,
    'role': role,
    'text': text,
    'createdAt': createdAt.toIso8601String(),
    'attachmentPaths': attachmentPaths,
  };

  factory ChatMessage.fromJson(Map<String, dynamic> json) => ChatMessage(
    id: json['id'] as String,
    role: json['role'] as String,
    text: json['text'] as String,
    createdAt: DateTime.parse(json['createdAt'] as String),
    attachmentPaths: (json['attachmentPaths'] as List<dynamic>? ?? const [])
        .map((value) => value.toString())
        .toList(),
  );
}

class ChatConversation {
  ChatConversation({
    required this.id,
    required this.title,
    required this.messages,
    required this.updatedAt,
  });

  final String id;
  String title;
  final List<ChatMessage> messages;
  DateTime updatedAt;

  Map<String, dynamic> toJson() => {
    'id': id,
    'title': title,
    'messages': messages.map((message) => message.toJson()).toList(),
    'updatedAt': updatedAt.toIso8601String(),
  };

  factory ChatConversation.fromJson(Map<String, dynamic> json) =>
      ChatConversation(
        id: json['id'] as String,
        title: json['title'] as String,
        messages: (json['messages'] as List<dynamic>)
            .map(
              (item) =>
                  ChatMessage.fromJson(Map<String, dynamic>.from(item as Map)),
            )
            .toList(),
        updatedAt: DateTime.parse(json['updatedAt'] as String),
      );
}

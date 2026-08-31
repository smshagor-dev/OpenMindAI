import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path/path.dart' as p;

import '../../core/constants/model_catalog.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/storage/onboarding_store.dart';
import '../models/model_manager_sheet.dart';
import 'models/chat_models.dart';
import 'services/chat_store.dart';
import 'services/mobile_inference_service.dart';

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _scaffoldKey = GlobalKey<ScaffoldState>();
  final _composer = TextEditingController();
  final _chatStore = ChatStore();
  final _onboardingStore = OnboardingStore();
  final _modelStorage = ModelStorageService();
  final _imagePicker = ImagePicker();

  late final NativeMobileInferenceService _inference;
  StreamSubscription<String>? _generationSubscription;

  List<ChatConversation> _conversations = [];
  String? _activeConversationId;
  String _selectedModelId = 'qwen3-06b-q4';
  String _mode = 'chat';
  bool _loading = true;
  bool _generating = false;
  final List<String> _attachmentPaths = [];

  ChatConversation? get _activeConversation {
    for (final conversation in _conversations) {
      if (conversation.id == _activeConversationId) return conversation;
    }
    return null;
  }

  MobileModel get _selectedModel => MobileModelCatalog.byId(_selectedModelId);

  @override
  void initState() {
    super.initState();
    _inference = NativeMobileInferenceService(storage: _modelStorage);
    _load();
  }

  @override
  void dispose() {
    _generationSubscription?.cancel();
    unawaited(_inference.cancel());
    _composer.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final conversations = await _chatStore.load();
    final selectedModelId = await _onboardingStore.selectedModelId();
    if (!mounted) return;
    setState(() {
      _conversations = conversations;
      _activeConversationId = conversations.isEmpty ? null : conversations.first.id;
      _selectedModelId = selectedModelId ?? _selectedModelId;
      _loading = false;
    });
  }

  String _id(String prefix) => '$prefix-${DateTime.now().microsecondsSinceEpoch}';

  Future<void> _newChat() async {
    if (_generating) await _stopGeneration();
    if (!mounted) return;
    setState(() {
      _activeConversationId = null;
      _attachmentPaths.clear();
      _composer.clear();
    });
  }

  ChatConversation _ensureConversation() {
    final current = _activeConversation;
    if (current != null) return current;
    final conversation = ChatConversation(
      id: _id('chat'),
      title: 'New chat',
      messages: [],
      updatedAt: DateTime.now(),
    );
    _conversations.insert(0, conversation);
    _activeConversationId = conversation.id;
    return conversation;
  }

  Future<void> _send() async {
    final text = _composer.text.trim();
    if (_generating || (text.isEmpty && _attachmentPaths.isEmpty)) return;

    final conversation = _ensureConversation();
    final attachments = List<String>.from(_attachmentPaths);
    final userText = text.isEmpty ? 'Review the attached content.' : text;
    final userMessage = ChatMessage(
      id: _id('message'),
      role: 'user',
      text: userText,
      createdAt: DateTime.now(),
      attachmentPaths: attachments,
    );

    setState(() {
      conversation.messages.add(userMessage);
      conversation.updatedAt = DateTime.now();
      if (conversation.title == 'New chat') {
        conversation.title = userText.length > 42 ? '${userText.substring(0, 42)}…' : userText;
      }
      _composer.clear();
      _attachmentPaths.clear();
      _conversations.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
    });
    await _chatStore.save(_conversations);
    await _beginGeneration(conversation, attachments: attachments);
  }

  Future<void> _beginGeneration(
    ChatConversation conversation, {
    required List<String> attachments,
  }) async {
    if (_generating) return;
    final requestMessages = List<ChatMessage>.from(conversation.messages);
    final assistantId = _id('message');
    final placeholder = ChatMessage(
      id: assistantId,
      role: 'assistant',
      text: '',
      createdAt: DateTime.now(),
    );

    setState(() {
      _generating = true;
      conversation.messages.add(placeholder);
      conversation.updatedAt = DateTime.now();
    });

    var finished = false;
    Future<void> finish() async {
      if (finished) return;
      finished = true;
      final index = conversation.messages.indexWhere((message) => message.id == assistantId);
      if (index >= 0 && conversation.messages[index].text.trim().isEmpty) {
        conversation.messages.removeAt(index);
      }
      conversation.updatedAt = DateTime.now();
      await _chatStore.save(_conversations);
      if (mounted) setState(() => _generating = false);
    }

    final stream = _inference.stream(
      MobileInferenceRequest(
        modelId: _selectedModelId,
        mode: _mode,
        messages: requestMessages,
        attachmentPaths: attachments,
      ),
    );

    _generationSubscription = stream.listen(
      (delta) {
        if (!mounted) return;
        final index = conversation.messages.indexWhere((message) => message.id == assistantId);
        if (index < 0) return;
        final current = conversation.messages[index];
        setState(() => conversation.messages[index] = current.copyWith(text: current.text + delta));
      },
      onError: (Object error) async {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(error.toString()),
              behavior: SnackBarBehavior.floating,
              action: SnackBarAction(label: 'Models', onPressed: _openModels),
            ),
          );
        }
        await finish();
      },
      onDone: finish,
      cancelOnError: true,
    );
  }

  Future<void> _stopGeneration() async {
    await _generationSubscription?.cancel();
    _generationSubscription = null;
    await _inference.cancel();
    if (_activeConversation != null) await _chatStore.save(_conversations);
    if (mounted) setState(() => _generating = false);
  }

  Future<void> _regenerate() async {
    if (_generating) return;
    final conversation = _activeConversation;
    if (conversation == null || conversation.messages.isEmpty) return;
    if (conversation.messages.last.role == 'assistant') {
      setState(() => conversation.messages.removeLast());
    }
    final lastUser = conversation.messages.lastWhere(
      (message) => message.role == 'user',
      orElse: () => ChatMessage(id: '', role: 'user', text: '', createdAt: DateTime.now()),
    );
    if (lastUser.id.isEmpty) return;
    await _beginGeneration(conversation, attachments: lastUser.attachmentPaths);
  }

  Future<void> _pickCamera() async {
    final image = await _imagePicker.pickImage(source: ImageSource.camera, imageQuality: 92);
    if (image != null && mounted) setState(() => _attachmentPaths.add(image.path));
  }

  Future<void> _pickPhotos() async {
    final images = await _imagePicker.pickMultiImage(imageQuality: 92);
    if (images.isNotEmpty && mounted) {
      setState(() => _attachmentPaths.addAll(images.map((image) => image.path)));
    }
  }

  Future<void> _pickFiles() async {
    final result = await FilePicker.platform.pickFiles(allowMultiple: true);
    if (result == null || !mounted) return;
    final paths = result.files.map((file) => file.path).whereType<String>();
    setState(() => _attachmentPaths.addAll(paths));
  }

  void _openAttachmentMenu() {
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(18, 0, 18, 18),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.camera_alt_outlined),
                title: const Text('Camera'),
                subtitle: const Text('Images automatically use OpenMindAI Lens'),
                onTap: () {
                  Navigator.pop(context);
                  _pickCamera();
                },
              ),
              ListTile(
                leading: const Icon(Icons.photo_library_outlined),
                title: const Text('Photos'),
                onTap: () {
                  Navigator.pop(context);
                  _pickPhotos();
                },
              ),
              ListTile(
                leading: const Icon(Icons.attach_file_rounded),
                title: const Text('Files'),
                subtitle: const Text('Text, code, JSON, Markdown, and other local files'),
                onTap: () {
                  Navigator.pop(context);
                  _pickFiles();
                },
              ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _selectModel() async {
    final selected = await showModalBottomSheet<String>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          padding: const EdgeInsets.fromLTRB(12, 0, 12, 24),
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 4, 8, 12),
              child: Row(
                children: [
                  const Expanded(child: Text('Choose model', style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700))),
                  TextButton.icon(
                    onPressed: () {
                      Navigator.pop(context);
                      _openModels();
                    },
                    icon: const Icon(Icons.download_rounded),
                    label: const Text('Manage'),
                  ),
                ],
              ),
            ),
            ...MobileModelCatalog.models.map((model) => ListTile(
                  title: Text(model.name, style: const TextStyle(fontWeight: FontWeight.w600)),
                  subtitle: Text('${model.kind} · ${model.minRamGb}+ GB RAM · ~${model.sizeGb.toStringAsFixed(1)} GB'),
                  trailing: model.id == _selectedModelId ? const Icon(Icons.check_rounded) : null,
                  onTap: () => Navigator.pop(context, model.id),
                )),
          ],
        ),
      ),
    );
    if (selected == null || !mounted) return;
    await _onboardingStore.setSelectedModelId(selected);
    if (mounted) setState(() => _selectedModelId = selected);
  }

  Future<void> _openModels() async {
    if (!mounted) return;
    await showModelManagerSheet(
      context,
      storage: _modelStorage,
      onModelReady: (modelId) async {
        await _onboardingStore.setSelectedModelId(modelId);
        if (mounted) setState(() => _selectedModelId = modelId);
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final conversation = _activeConversation;
    return Scaffold(
      key: _scaffoldKey,
      drawer: _buildDrawer(context),
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(Icons.menu_rounded),
          onPressed: () => _scaffoldKey.currentState?.openDrawer(),
        ),
        titleSpacing: 2,
        title: InkWell(
          borderRadius: BorderRadius.circular(12),
          onTap: _selectModel,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              Flexible(child: Text(_selectedModel.name, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600))),
              const SizedBox(width: 3),
              const Icon(Icons.keyboard_arrow_down_rounded, size: 20),
            ]),
          ),
        ),
        actions: [
          IconButton(onPressed: _openModels, icon: const Icon(Icons.memory_rounded), tooltip: 'Models'),
          IconButton(onPressed: _newChat, icon: const Icon(Icons.edit_square), tooltip: 'New chat'),
          const SizedBox(width: 4),
        ],
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator(strokeWidth: 2))
          : Column(
              children: [
                Expanded(
                  child: conversation == null || conversation.messages.isEmpty
                      ? _EmptyChat(modelName: _selectedModel.name, onModels: _openModels)
                      : ListView.builder(
                          padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
                          itemCount: conversation.messages.length,
                          itemBuilder: (context, index) {
                            final message = conversation.messages[index];
                            final isLastAssistant = message.role == 'assistant' && index == conversation.messages.length - 1;
                            return _MessageBubble(
                              message: message,
                              onRegenerate: isLastAssistant && !_generating ? _regenerate : null,
                            );
                          },
                        ),
                ),
                _Composer(
                  controller: _composer,
                  mode: _mode,
                  attachmentPaths: _attachmentPaths,
                  generating: _generating,
                  onModeChanged: (value) => setState(() => _mode = value),
                  onAdd: _openAttachmentMenu,
                  onSend: _send,
                  onStop: _stopGeneration,
                  onRemoveAttachment: (path) => setState(() => _attachmentPaths.remove(path)),
                ),
              ],
            ),
    );
  }

  Widget _buildDrawer(BuildContext context) {
    return Drawer(
      width: MediaQuery.sizeOf(context).width * .86,
      child: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
              child: Row(children: [
                const Expanded(child: Text('OpenMindAI', style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700))),
                IconButton(
                  onPressed: () {
                    Navigator.pop(context);
                    _newChat();
                  },
                  icon: const Icon(Icons.edit_square),
                ),
              ]),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: TextField(
                decoration: const InputDecoration(prefixIcon: Icon(Icons.search_rounded), hintText: 'Search chats', isDense: true),
                onChanged: (_) => setState(() {}),
              ),
            ),
            const SizedBox(height: 10),
            Expanded(
              child: _conversations.isEmpty
                  ? const Center(child: Text('No conversations yet'))
                  : ListView.builder(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      itemCount: _conversations.length,
                      itemBuilder: (context, index) {
                        final item = _conversations[index];
                        return ListTile(
                          dense: true,
                          selected: item.id == _activeConversationId,
                          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                          title: Text(item.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                          onTap: () {
                            setState(() => _activeConversationId = item.id);
                            Navigator.pop(context);
                          },
                        );
                      },
                    ),
            ),
            const Divider(height: 1),
            ListTile(
              leading: const Icon(Icons.memory_rounded),
              title: const Text('Models'),
              subtitle: const Text('Download, verify, and remove local models'),
              onTap: () {
                Navigator.pop(context);
                _openModels();
              },
            ),
            ListTile(
              leading: const CircleAvatar(child: Icon(Icons.person_outline_rounded)),
              title: const Text('OpenMindAI Mobile'),
              subtitle: const Text('Local-first'),
              trailing: const Icon(Icons.more_horiz_rounded),
              onTap: () {},
            ),
          ],
        ),
      ),
    );
  }
}

class _EmptyChat extends StatelessWidget {
  const _EmptyChat({required this.modelName, required this.onModels});
  final String modelName;
  final VoidCallback onModels;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 32),
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          Container(
            width: 54,
            height: 54,
            decoration: BoxDecoration(color: Theme.of(context).colorScheme.onSurface, shape: BoxShape.circle),
            child: Icon(Icons.psychology_alt_rounded, color: Theme.of(context).colorScheme.surface, size: 31),
          ),
          const SizedBox(height: 18),
          Text('How can I help?', style: Theme.of(context).textTheme.headlineSmall?.copyWith(fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          Text(modelName, style: Theme.of(context).textTheme.bodySmall),
          const SizedBox(height: 16),
          TextButton.icon(onPressed: onModels, icon: const Icon(Icons.download_rounded), label: const Text('Manage local models')),
        ]),
      ),
    );
  }
}

class _MessageBubble extends StatelessWidget {
  const _MessageBubble({required this.message, this.onRegenerate});
  final ChatMessage message;
  final VoidCallback? onRegenerate;

  @override
  Widget build(BuildContext context) {
    final user = message.role == 'user';
    return Align(
      alignment: user ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints: BoxConstraints(maxWidth: MediaQuery.sizeOf(context).width * (user ? .82 : .94)),
        margin: const EdgeInsets.only(bottom: 18),
        padding: user ? const EdgeInsets.symmetric(horizontal: 16, vertical: 11) : EdgeInsets.zero,
        decoration: user
            ? BoxDecoration(
                color: Theme.of(context).brightness == Brightness.dark ? const Color(0xFF303030) : const Color(0xFFF1F1F1),
                borderRadius: BorderRadius.circular(20),
              )
            : null,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (message.attachmentPaths.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: message.attachmentPaths
                      .map((path) => Chip(
                            visualDensity: VisualDensity.compact,
                            avatar: const Icon(Icons.attach_file_rounded, size: 15),
                            label: Text(p.basename(path), overflow: TextOverflow.ellipsis),
                          ))
                      .toList(),
                ),
              ),
            if (message.text.isEmpty && !user)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 4),
                child: Row(mainAxisSize: MainAxisSize.min, children: [
                  SizedBox(width: 17, height: 17, child: CircularProgressIndicator(strokeWidth: 2)),
                  SizedBox(width: 9),
                  Text('Thinking…'),
                ]),
              )
            else
              SelectableText(message.text, style: const TextStyle(fontSize: 16, height: 1.45)),
            if (!user && message.text.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      visualDensity: VisualDensity.compact,
                      tooltip: 'Copy',
                      onPressed: () => Clipboard.setData(ClipboardData(text: message.text)),
                      icon: const Icon(Icons.copy_rounded, size: 18),
                    ),
                    if (onRegenerate != null)
                      IconButton(
                        visualDensity: VisualDensity.compact,
                        tooltip: 'Regenerate',
                        onPressed: onRegenerate,
                        icon: const Icon(Icons.refresh_rounded, size: 19),
                      ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller,
    required this.mode,
    required this.attachmentPaths,
    required this.generating,
    required this.onModeChanged,
    required this.onAdd,
    required this.onSend,
    required this.onStop,
    required this.onRemoveAttachment,
  });

  final TextEditingController controller;
  final String mode;
  final List<String> attachmentPaths;
  final bool generating;
  final ValueChanged<String> onModeChanged;
  final VoidCallback onAdd;
  final VoidCallback onSend;
  final VoidCallback onStop;
  final ValueChanged<String> onRemoveAttachment;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 4, 10, 10),
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(children: [
              _ModeChip(label: 'Chat', value: 'chat', selected: mode == 'chat', onSelected: onModeChanged),
              _ModeChip(label: 'Think', value: 'thinking', selected: mode == 'thinking', onSelected: onModeChanged),
              _ModeChip(label: 'Search', value: 'web-search', selected: mode == 'web-search', onSelected: onModeChanged),
              _ModeChip(label: 'Research', value: 'research', selected: mode == 'research', onSelected: onModeChanged),
            ]),
          ),
          if (attachmentPaths.isNotEmpty)
            Align(
              alignment: Alignment.centerLeft,
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Row(
                  children: attachmentPaths
                      .map((path) => Padding(
                            padding: const EdgeInsets.only(right: 6, bottom: 6),
                            child: Chip(
                              avatar: const Icon(Icons.attach_file_rounded, size: 17),
                              label: Text(p.basename(path)),
                              deleteIcon: const Icon(Icons.close_rounded, size: 17),
                              onDeleted: () => onRemoveAttachment(path),
                            ),
                          ))
                      .toList(),
                ),
              ),
            ),
          TextField(
            controller: controller,
            minLines: 1,
            maxLines: 6,
            textInputAction: TextInputAction.newline,
            decoration: InputDecoration(
              hintText: 'Message OpenMindAI',
              contentPadding: const EdgeInsets.symmetric(vertical: 11),
              prefixIcon: IconButton(onPressed: generating ? null : onAdd, icon: const Icon(Icons.add_rounded)),
              suffixIcon: Padding(
                padding: const EdgeInsets.all(6),
                child: IconButton.filled(
                  onPressed: generating ? onStop : onSend,
                  icon: Icon(generating ? Icons.stop_rounded : Icons.arrow_upward_rounded),
                ),
              ),
            ),
            onSubmitted: (_) {
              if (!generating) onSend();
            },
          ),
          const SizedBox(height: 5),
          Text(
            'OpenMindAI can make mistakes. Check important information.',
            style: Theme.of(context).textTheme.labelSmall,
          ),
        ]),
      ),
    );
  }
}

class _ModeChip extends StatelessWidget {
  const _ModeChip({required this.label, required this.value, required this.selected, required this.onSelected});
  final String label;
  final String value;
  final bool selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 7, bottom: 7),
      child: ChoiceChip(
        label: Text(label),
        selected: selected,
        onSelected: (_) => onSelected(value),
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}

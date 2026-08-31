import 'dart:async';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path/path.dart' as p;

import '../../core/constants/model_catalog.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/storage/onboarding_store.dart';
import '../models/model_manager_sheet.dart';
import 'models/chat_models.dart';
import 'services/chat_store.dart';
import 'services/mobile_inference_service.dart';
import 'services/voice_input_service.dart';

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _scaffoldKey = GlobalKey<ScaffoldState>();
  final _composer = TextEditingController();
  final _searchController = TextEditingController();
  final _scrollController = ScrollController();
  final _chatStore = ChatStore();
  final _onboardingStore = OnboardingStore();
  final _modelStorage = ModelStorageService();
  final _imagePicker = ImagePicker();
  final _voice = VoiceInputService();

  late final NativeMobileInferenceService _inference;
  StreamSubscription<String>? _generationSubscription;
  StreamSubscription<VoiceTranscriptEvent>? _voiceSubscription;

  List<ChatConversation> _conversations = [];
  final List<String> _attachmentPaths = [];
  String? _activeConversationId;
  String? _activeAssistantId;
  String _selectedModelId = 'qwen3-06b-q4';
  String _mode = 'chat';
  String _chatSearchQuery = '';
  String _voiceBaseText = '';
  bool _loading = true;
  bool _generating = false;
  bool _voiceListening = false;
  bool _voicePreparing = false;

  ChatConversation? get _activeConversation {
    for (final conversation in _conversations) {
      if (conversation.id == _activeConversationId) return conversation;
    }
    return null;
  }

  MobileModel get _selectedModel => MobileModelCatalog.byId(_selectedModelId);

  List<ChatConversation> get _visibleConversations {
    final query = _chatSearchQuery.trim().toLowerCase();
    if (query.isEmpty) return _conversations;
    return _conversations.where((conversation) {
      if (conversation.title.toLowerCase().contains(query)) return true;
      return conversation.messages.any(
        (message) => message.text.toLowerCase().contains(query),
      );
    }).toList();
  }

  @override
  void initState() {
    super.initState();
    _inference = NativeMobileInferenceService(storage: _modelStorage);
    _voiceSubscription = _voice.events.listen(
      (event) {
        if (!mounted) return;
        _applyVoiceText(event.text);
        if (event.isFinal) setState(() => _voiceListening = false);
      },
      onError: (Object error) {
        if (!mounted) return;
        setState(() {
          _voiceListening = false;
          _voicePreparing = false;
        });
        _showError('OpenMindAI Hear could not continue. Please try again.');
      },
    );
    _load();
  }

  @override
  void dispose() {
    _generationSubscription?.cancel();
    _voiceSubscription?.cancel();
    unawaited(_inference.shutdown());
    unawaited(_voice.dispose());
    _composer.dispose();
    _searchController.dispose();
    _scrollController.dispose();
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
    _scrollToBottom();
  }

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scrollController.hasClients) return;
      _scrollController.animateTo(
        _scrollController.position.maxScrollExtent,
        duration: const Duration(milliseconds: 180),
        curve: Curves.easeOut,
      );
    });
  }

  String _id(String prefix) => '$prefix-${DateTime.now().microsecondsSinceEpoch}';

  void _showError(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(message), behavior: SnackBarBehavior.floating),
    );
  }

  void _applyVoiceText(String transcript) {
    final normalized = transcript.trim();
    if (normalized.isEmpty) return;
    final prefix = _voiceBaseText.trimRight();
    final value = prefix.isEmpty ? normalized : '$prefix $normalized';
    _composer.value = TextEditingValue(
      text: value,
      selection: TextSelection.collapsed(offset: value.length),
    );
  }

  Future<void> _toggleVoice() async {
    if (_generating || _voicePreparing) return;
    if (_voiceListening) {
      setState(() => _voicePreparing = true);
      try {
        final finalText = await _voice.stop();
        if (mounted && finalText.trim().isNotEmpty) _applyVoiceText(finalText);
      } catch (_) {
        _showError('OpenMindAI Hear could not finish the dictation.');
      } finally {
        if (mounted) {
          setState(() {
            _voiceListening = false;
            _voicePreparing = false;
          });
        }
      }
      return;
    }

    _voiceBaseText = _composer.text;
    setState(() => _voicePreparing = true);
    try {
      await _voice.start();
      if (mounted) setState(() => _voiceListening = true);
    } on VoiceInputException catch (error) {
      _showError(error.message);
    } catch (_) {
      _showError(
        'OpenMindAI Hear could not start. Check microphone permission and try again.',
      );
    } finally {
      if (mounted) setState(() => _voicePreparing = false);
    }
  }

  Future<void> _stopVoiceIfNeeded() async {
    if (!_voiceListening) return;
    try {
      final finalText = await _voice.stop();
      if (mounted && finalText.trim().isNotEmpty) _applyVoiceText(finalText);
    } finally {
      if (mounted) setState(() => _voiceListening = false);
    }
  }

  Future<void> _newChat() async {
    if (_generating) await _stopGeneration();
    if (_voiceListening) await _voice.cancel();
    if (!mounted) return;
    setState(() {
      _voiceListening = false;
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
    if (_generating) return;
    await _stopVoiceIfNeeded();
    final text = _composer.text.trim();
    if (text.isEmpty && _attachmentPaths.isEmpty) return;

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
        conversation.title = userText.length > 42
            ? '${userText.substring(0, 42)}…'
            : userText;
      }
      _composer.clear();
      _attachmentPaths.clear();
      _conversations.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
    });
    await _chatStore.save(_conversations);
    _scrollToBottom();
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
      _activeAssistantId = assistantId;
      conversation.messages.add(placeholder);
      conversation.updatedAt = DateTime.now();
    });
    _scrollToBottom();

    var finished = false;
    Future<void> finish() async {
      if (finished) return;
      finished = true;
      _generationSubscription = null;
      if (_activeAssistantId == assistantId) _activeAssistantId = null;
      final index = conversation.messages.indexWhere(
        (message) => message.id == assistantId,
      );
      if (index >= 0 && conversation.messages[index].text.trim().isEmpty) {
        conversation.messages.removeAt(index);
      }
      conversation.updatedAt = DateTime.now();
      await _chatStore.save(_conversations);
      if (mounted) setState(() => _generating = false);
      _scrollToBottom();
    }

    _generationSubscription = _inference
        .stream(
          MobileInferenceRequest(
            modelId: _selectedModelId,
            mode: _mode,
            messages: requestMessages,
            attachmentPaths: attachments,
          ),
        )
        .listen(
          (delta) {
            if (!mounted) return;
            final index = conversation.messages.indexWhere(
              (message) => message.id == assistantId,
            );
            if (index < 0) return;
            final current = conversation.messages[index];
            setState(() {
              conversation.messages[index] = current.copyWith(
                text: current.text + delta,
              );
            });
            _scrollToBottom();
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
    final conversation = _activeConversation;
    final assistantId = _activeAssistantId;
    _activeAssistantId = null;
    if (conversation != null && assistantId != null) {
      final index = conversation.messages.indexWhere(
        (message) => message.id == assistantId,
      );
      if (index >= 0 && conversation.messages[index].text.trim().isEmpty) {
        conversation.messages.removeAt(index);
      }
      conversation.updatedAt = DateTime.now();
      await _chatStore.save(_conversations);
    }
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
      orElse: () => ChatMessage(
        id: '',
        role: 'user',
        text: '',
        createdAt: DateTime.now(),
      ),
    );
    if (lastUser.id.isEmpty) return;
    await _chatStore.save(_conversations);
    await _beginGeneration(conversation, attachments: lastUser.attachmentPaths);
  }

  Future<void> _pickCamera() async {
    final image = await _imagePicker.pickImage(
      source: ImageSource.camera,
      imageQuality: 92,
    );
    if (image != null && mounted) {
      setState(() => _attachmentPaths.add(image.path));
    }
  }

  Future<void> _pickPhotos() async {
    final images = await _imagePicker.pickMultiImage(imageQuality: 92);
    if (images.isNotEmpty && mounted) {
      setState(() => _attachmentPaths.addAll(images.map((image) => image.path)));
    }
  }

  Future<void> _pickFiles() async {
    final files = await FilePicker.pickFiles();
    if (files.isEmpty || !mounted) return;
    final paths = files
        .map((file) => file.xFile.path)
        .where((path) => path.isNotEmpty);
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
                subtitle: const Text('PDF, text, code, JSON, Markdown, YAML and CSV'),
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
                  const Expanded(
                    child: Text(
                      'Choose model',
                      style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
                    ),
                  ),
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
            ...MobileModelCatalog.models.map(
              (model) => ListTile(
                title: Text(
                  model.name,
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
                subtitle: Text(
                  '${model.kind} · ${model.minRamGb}+ GB RAM · ~${model.sizeGb.toStringAsFixed(1)} GB',
                ),
                trailing: model.id == _selectedModelId
                    ? const Icon(Icons.check_rounded)
                    : null,
                onTap: () => Navigator.pop(context, model.id),
              ),
            ),
          ],
        ),
      ),
    );
    if (selected == null || !mounted) return;
    if (selected != _selectedModelId) await _inference.shutdown();
    await _onboardingStore.setSelectedModelId(selected);
    if (mounted) setState(() => _selectedModelId = selected);
  }

  Future<void> _openModels() async {
    if (!mounted) return;
    await showModelManagerSheet(
      context,
      storage: _modelStorage,
      onModelReady: (modelId) async {
        if (modelId != _selectedModelId) await _inference.shutdown();
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
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Flexible(
                  child: Text(
                    _selectedModel.name,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600),
                  ),
                ),
                const SizedBox(width: 3),
                const Icon(Icons.keyboard_arrow_down_rounded, size: 20),
              ],
            ),
          ),
        ),
        actions: [
          IconButton(
            onPressed: _openModels,
            icon: const Icon(Icons.memory_rounded),
            tooltip: 'Models',
          ),
          IconButton(
            onPressed: _newChat,
            icon: const Icon(Icons.edit_square),
            tooltip: 'New chat',
          ),
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
                          controller: _scrollController,
                          padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
                          itemCount: conversation.messages.length,
                          itemBuilder: (context, index) {
                            final message = conversation.messages[index];
                            final lastAssistant = message.role == 'assistant' &&
                                index == conversation.messages.length - 1;
                            return _MessageBubble(
                              message: message,
                              onRegenerate: lastAssistant && !_generating
                                  ? _regenerate
                                  : null,
                            );
                          },
                        ),
                ),
                _Composer(
                  controller: _composer,
                  mode: _mode,
                  attachmentPaths: _attachmentPaths,
                  generating: _generating,
                  voiceListening: _voiceListening,
                  voicePreparing: _voicePreparing,
                  onModeChanged: (value) => setState(() => _mode = value),
                  onAdd: _openAttachmentMenu,
                  onVoice: _toggleVoice,
                  onSend: _send,
                  onStop: _stopGeneration,
                  onRemoveAttachment: (path) {
                    setState(() => _attachmentPaths.remove(path));
                  },
                ),
              ],
            ),
    );
  }

  Widget _buildDrawer(BuildContext context) {
    final visibleConversations = _visibleConversations;
    return Drawer(
      width: MediaQuery.sizeOf(context).width * .86,
      child: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
              child: Row(
                children: [
                  const Expanded(
                    child: Text(
                      'OpenMindAI',
                      style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
                    ),
                  ),
                  IconButton(
                    onPressed: () {
                      Navigator.pop(context);
                      _newChat();
                    },
                    icon: const Icon(Icons.edit_square),
                  ),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: TextField(
                controller: _searchController,
                decoration: InputDecoration(
                  prefixIcon: const Icon(Icons.search_rounded),
                  hintText: 'Search chats',
                  isDense: true,
                  suffixIcon: _chatSearchQuery.isEmpty
                      ? null
                      : IconButton(
                          onPressed: () {
                            _searchController.clear();
                            setState(() => _chatSearchQuery = '');
                          },
                          icon: const Icon(Icons.close_rounded),
                        ),
                ),
                onChanged: (value) => setState(() => _chatSearchQuery = value),
              ),
            ),
            const SizedBox(height: 10),
            Expanded(
              child: visibleConversations.isEmpty
                  ? Center(
                      child: Text(
                        _chatSearchQuery.isEmpty
                            ? 'No conversations yet'
                            : 'No matching chats',
                      ),
                    )
                  : ListView.builder(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      itemCount: visibleConversations.length,
                      itemBuilder: (context, index) {
                        final item = visibleConversations[index];
                        return ListTile(
                          dense: true,
                          selected: item.id == _activeConversationId,
                          shape: RoundedRectangleBorder(
                            borderRadius: BorderRadius.circular(10),
                          ),
                          title: Text(
                            item.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                          onTap: () {
                            setState(() => _activeConversationId = item.id);
                            Navigator.pop(context);
                            _scrollToBottom();
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
            const ListTile(
              leading: CircleAvatar(child: Icon(Icons.person_outline_rounded)),
              title: Text('OpenMindAI Mobile'),
              subtitle: Text('Local-first'),
              trailing: Icon(Icons.more_horiz_rounded),
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
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 54,
              height: 54,
              decoration: BoxDecoration(
                color: Theme.of(context).colorScheme.onSurface,
                shape: BoxShape.circle,
              ),
              child: Icon(
                Icons.psychology_alt_rounded,
                color: Theme.of(context).colorScheme.surface,
                size: 31,
              ),
            ),
            const SizedBox(height: 18),
            Text(
              'How can I help?',
              style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 8),
            Text(modelName, style: Theme.of(context).textTheme.bodySmall),
            const SizedBox(height: 16),
            TextButton.icon(
              onPressed: onModels,
              icon: const Icon(Icons.download_rounded),
              label: const Text('Manage local models'),
            ),
          ],
        ),
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
        constraints: BoxConstraints(
          maxWidth: MediaQuery.sizeOf(context).width * (user ? .84 : .96),
        ),
        margin: const EdgeInsets.only(bottom: 18),
        padding: user
            ? const EdgeInsets.symmetric(horizontal: 16, vertical: 11)
            : EdgeInsets.zero,
        decoration: user
            ? BoxDecoration(
                color: Theme.of(context).brightness == Brightness.dark
                    ? const Color(0xFF303030)
                    : const Color(0xFFF1F1F1),
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
                  spacing: 7,
                  runSpacing: 7,
                  children: message.attachmentPaths
                      .map((path) => _AttachmentPreview(path: path))
                      .toList(),
                ),
              ),
            if (message.text.isEmpty && !user)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SizedBox(
                      width: 17,
                      height: 17,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    SizedBox(width: 9),
                    Text('Thinking…'),
                  ],
                ),
              )
            else if (user)
              SelectableText(
                message.text,
                style: const TextStyle(fontSize: 16, height: 1.45),
              )
            else
              MarkdownBody(
                data: message.text,
                selectable: true,
                styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context)).copyWith(
                  p: const TextStyle(fontSize: 16, height: 1.45),
                  code: TextStyle(
                    fontFamily: 'monospace',
                    fontSize: 14,
                    backgroundColor:
                        Theme.of(context).colorScheme.surfaceContainerHighest,
                  ),
                  codeblockDecoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surfaceContainerHighest,
                    borderRadius: BorderRadius.circular(12),
                  ),
                ),
              ),
            if (!user && message.text.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      visualDensity: VisualDensity.compact,
                      tooltip: 'Copy',
                      onPressed: () => Clipboard.setData(
                        ClipboardData(text: message.text),
                      ),
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

class _AttachmentPreview extends StatelessWidget {
  const _AttachmentPreview({required this.path});

  final String path;

  bool get _isImage => const {'.png', '.jpg', '.jpeg', '.webp'}.contains(
        p.extension(path).toLowerCase(),
      );

  @override
  Widget build(BuildContext context) {
    if (_isImage) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(14),
        child: Image.file(
          File(path),
          width: 160,
          height: 120,
          fit: BoxFit.cover,
          errorBuilder: (context, error, stackTrace) => _fileChip(),
        ),
      );
    }
    return _fileChip();
  }

  Widget _fileChip() => Chip(
        visualDensity: VisualDensity.compact,
        avatar: const Icon(Icons.attach_file_rounded, size: 15),
        label: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 190),
          child: Text(p.basename(path), overflow: TextOverflow.ellipsis),
        ),
      );
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller,
    required this.mode,
    required this.attachmentPaths,
    required this.generating,
    required this.voiceListening,
    required this.voicePreparing,
    required this.onModeChanged,
    required this.onAdd,
    required this.onVoice,
    required this.onSend,
    required this.onStop,
    required this.onRemoveAttachment,
  });

  final TextEditingController controller;
  final String mode;
  final List<String> attachmentPaths;
  final bool generating;
  final bool voiceListening;
  final bool voicePreparing;
  final ValueChanged<String> onModeChanged;
  final VoidCallback onAdd;
  final VoidCallback onVoice;
  final VoidCallback onSend;
  final VoidCallback onStop;
  final ValueChanged<String> onRemoveAttachment;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 4, 10, 10),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  _ModeChip(
                    label: 'Chat',
                    value: 'chat',
                    selected: mode == 'chat',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Think',
                    value: 'thinking',
                    selected: mode == 'thinking',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Search',
                    value: 'web-search',
                    selected: mode == 'web-search',
                    onSelected: onModeChanged,
                  ),
                  _ModeChip(
                    label: 'Research',
                    value: 'research',
                    selected: mode == 'research',
                    onSelected: onModeChanged,
                  ),
                ],
              ),
            ),
            if (attachmentPaths.isNotEmpty)
              Align(
                alignment: Alignment.centerLeft,
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: attachmentPaths
                        .map(
                          (path) => Padding(
                            padding: const EdgeInsets.only(right: 6, bottom: 6),
                            child: Chip(
                              avatar: const Icon(Icons.attach_file_rounded, size: 17),
                              label: Text(p.basename(path)),
                              deleteIcon: const Icon(Icons.close_rounded, size: 17),
                              onDeleted: () => onRemoveAttachment(path),
                            ),
                          ),
                        )
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
                hintText: voiceListening
                    ? 'Listening with OpenMindAI Hear…'
                    : 'Message OpenMindAI',
                contentPadding: const EdgeInsets.symmetric(vertical: 11),
                prefixIcon: IconButton(
                  onPressed: generating ? null : onAdd,
                  icon: const Icon(Icons.add_rounded),
                ),
                suffixIconConstraints: const BoxConstraints(minWidth: 96),
                suffixIcon: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: voiceListening
                          ? 'Stop OpenMindAI Hear'
                          : 'Voice input',
                      onPressed: generating || voicePreparing ? null : onVoice,
                      icon: voicePreparing
                          ? const SizedBox(
                              width: 19,
                              height: 19,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            )
                          : Icon(
                              voiceListening
                                  ? Icons.stop_circle_outlined
                                  : Icons.mic_none_rounded,
                            ),
                    ),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(0, 6, 6, 6),
                      child: IconButton.filled(
                        onPressed: generating ? onStop : onSend,
                        icon: Icon(
                          generating ? Icons.stop_rounded : Icons.arrow_upward_rounded,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
              onSubmitted: (_) {
                if (!generating) onSend();
              },
            ),
            const SizedBox(height: 5),
            Text(
              voiceListening
                  ? 'Listening locally · OpenMindAI Hear'
                  : 'OpenMindAI can make mistakes. Check important information.',
              style: Theme.of(context).textTheme.labelSmall,
            ),
          ],
        ),
      ),
    );
  }
}

class _ModeChip extends StatelessWidget {
  const _ModeChip({
    required this.label,
    required this.value,
    required this.selected,
    required this.onSelected,
  });

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

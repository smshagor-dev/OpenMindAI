import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:image_picker/image_picker.dart';

import '../../core/constants/model_catalog.dart';
import '../../core/services/attachment_storage_service.dart';
import '../../core/services/model_router_service.dart';
import '../../core/services/model_storage_service.dart';
import '../../core/services/notification_service.dart';
import '../../core/storage/app_settings_controller.dart';
import '../../core/storage/onboarding_store.dart';
import '../../core/theme/app_theme.dart';
import '../../core/theme/openmind_ui.dart';
import '../canvas/canvas_screen.dart';
import '../models/model_manager_sheet.dart';
import '../settings/settings_screen.dart';
import 'models/chat_models.dart';
import 'services/chat_store.dart';
import 'services/mobile_inference_service.dart';
import 'services/speech_output_service.dart';
import 'services/voice_input_service.dart';
import 'widgets/chat_composer.dart';
import 'widgets/chat_message_bubble.dart';

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
  final _attachmentStorage = AttachmentStorageService();
  final _onboardingStore = OnboardingStore();
  final _modelStorage = ModelStorageService();
  final _settings = AppSettingsController.instance;
  final _notifications = NotificationService.instance;
  final _imagePicker = ImagePicker();
  final _voice = VoiceInputService();
  final _speech = SpeechOutputService();

  late final MobileModelRouter _modelRouter;
  late final NativeMobileInferenceService _inference;
  StreamSubscription<String>? _generationSubscription;
  StreamSubscription<VoiceTranscriptEvent>? _voiceSubscription;

  List<ChatConversation> _conversations = [];
  final List<String> _attachmentPaths = [];
  String? _activeConversationId;
  String? _activeAssistantId;
  String? _speakingMessageId;
  String _selectedModelId = 'qwen3-06b-q4';
  String _mode = 'chat';
  String _chatSearchQuery = '';
  String _voiceBaseText = '';
  int _speechSession = 0;
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
    _modelRouter = MobileModelRouter(storage: _modelStorage);
    _inference = NativeMobileInferenceService(
      storage: _modelStorage,
      router: _modelRouter,
    );
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
    unawaited(_speech.dispose());
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
      _activeConversationId = conversations.isEmpty
          ? null
          : conversations.first.id;
      _selectedModelId = selectedModelId ?? _selectedModelId;
      _loading = false;
    });
    _scrollToBottom();
  }

  Future<void> _reloadHistory() async {
    final conversations = await _chatStore.load();
    if (!mounted) return;
    setState(() {
      _conversations = conversations;
      if (conversations.every((item) => item.id != _activeConversationId)) {
        _activeConversationId = conversations.isEmpty
            ? null
            : conversations.first.id;
      }
    });
    _scrollToBottom();
  }

  void _haptic({bool strong = false}) {
    if (!_settings.haptics) return;
    if (strong) {
      HapticFeedback.mediumImpact();
    } else {
      HapticFeedback.selectionClick();
    }
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

  String _id(String prefix) =>
      '$prefix-${DateTime.now().microsecondsSinceEpoch}';

  void _showError(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
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
    _haptic();
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

  Future<void> _stopSpeech() async {
    _speechSession += 1;
    await _speech.stop();
    if (mounted && _speakingMessageId != null) {
      setState(() => _speakingMessageId = null);
    }
  }

  Future<void> _toggleSpeech(ChatMessage message) async {
    if (message.role != 'assistant' || message.text.trim().isEmpty) return;
    _haptic();
    if (_speakingMessageId == message.id) {
      await _stopSpeech();
      return;
    }

    final session = ++_speechSession;
    await _speech.stop();
    if (!mounted || session != _speechSession) return;
    setState(() => _speakingMessageId = message.id);

    try {
      await _speech.speak(message.text);
    } on SpeechOutputException catch (error) {
      if (session == _speechSession) _showError(error.message);
    } catch (_) {
      if (session == _speechSession) {
        _showError(
          'OpenMindAI Speak is unavailable. Check that a device voice is installed.',
        );
      }
    } finally {
      if (mounted &&
          session == _speechSession &&
          _speakingMessageId == message.id) {
        setState(() => _speakingMessageId = null);
      }
    }
  }

  Future<void> _newChat() async {
    _haptic();
    if (_generating) await _stopGeneration();
    if (_voiceListening) await _voice.cancel();
    await _stopSpeech();
    if (!mounted) return;
    setState(() {
      _voiceListening = false;
      _activeConversationId = null;
      _attachmentPaths.clear();
      _composer.clear();
      _mode = 'chat';
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
    await _stopSpeech();
    final text = _composer.text.trim();
    if (text.isEmpty && _attachmentPaths.isEmpty) return;

    List<String> attachments;
    try {
      attachments = await _attachmentStorage.persistPaths(_attachmentPaths);
    } catch (error) {
      _showError('Could not save the selected attachment locally: $error');
      return;
    }
    if (text.isEmpty && attachments.isEmpty) {
      _showError('The selected attachment is no longer available.');
      return;
    }
    if (!await _ensureModelReadyForSend(attachments)) return;

    _haptic(strong: true);
    final conversation = _ensureConversation();
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
    Future<void> finish({required bool successful}) async {
      if (finished) return;
      finished = true;
      _generationSubscription = null;
      if (_activeAssistantId == assistantId) _activeAssistantId = null;
      final index = conversation.messages.indexWhere(
        (message) => message.id == assistantId,
      );
      final hasResponse =
          index >= 0 && conversation.messages[index].text.trim().isNotEmpty;
      if (index >= 0 && !hasResponse) {
        conversation.messages.removeAt(index);
      }
      conversation.updatedAt = DateTime.now();
      await _chatStore.save(_conversations);
      if (mounted) setState(() => _generating = false);
      _scrollToBottom();

      if (successful &&
          hasResponse &&
          _settings.completionNotifications &&
          WidgetsBinding.instance.lifecycleState != AppLifecycleState.resumed) {
        try {
          await _notifications.showResponseReady(
            conversationTitle: conversation.title,
          );
        } catch (_) {
          // Notification delivery is optional; the completed answer is already saved.
        }
      }
    }

    _generationSubscription = _inference
        .stream(
          MobileInferenceRequest(
            preferredModelId: _selectedModelId,
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
              final inferenceError = error is MobileInferenceException
                  ? error
                  : null;
              final message = inferenceError?.message ?? error.toString();
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(
                  content: Text(message),
                  action: inferenceError?.shouldOpenModels ?? true
                      ? SnackBarAction(label: 'Models', onPressed: _openModels)
                      : null,
                ),
              );
            }
            await finish(successful: false);
          },
          onDone: () async => finish(successful: true),
          cancelOnError: true,
        );
  }

  Future<bool> _ensureModelReadyForSend(List<String> attachments) async {
    try {
      final selection = await _modelRouter.resolve(
        requestedModelId: _selectedModelId,
        needsVision: _hasImageAttachment(attachments),
        taskType: _mode,
      );
      if (selection.model.id != _selectedModelId) {
        await _inference.shutdown();
        await _onboardingStore.setSelectedModelId(selection.model.id);
        if (mounted) setState(() => _selectedModelId = selection.model.id);
      }
      return true;
    } on MobileModelRoutingException catch (error) {
      if (!mounted) return false;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(error.message),
          action: SnackBarAction(label: 'Models', onPressed: _openModels),
        ),
      );
      return false;
    }
  }

  bool _hasImageAttachment(List<String> paths) {
    return paths.any((path) {
      final value = path.toLowerCase();
      return value.endsWith('.png') ||
          value.endsWith('.jpg') ||
          value.endsWith('.jpeg') ||
          value.endsWith('.webp');
    });
  }

  Future<void> _stopGeneration() async {
    _haptic();
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
    _haptic();
    await _stopSpeech();
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
    _haptic();
    final image = await _imagePicker.pickImage(
      source: ImageSource.camera,
      imageQuality: 92,
    );
    if (image != null && mounted) {
      setState(() => _attachmentPaths.add(image.path));
    }
  }

  Future<void> _pickPhotos() async {
    _haptic();
    final images = await _imagePicker.pickMultiImage(imageQuality: 92);
    if (images.isNotEmpty && mounted) {
      setState(
        () => _attachmentPaths.addAll(images.map((image) => image.path)),
      );
    }
  }

  Future<void> _pickFiles() async {
    _haptic();
    final files = await FilePicker.pickFiles();
    if (files.isEmpty || !mounted) return;
    final paths = files
        .map((file) => file.xFile.path)
        .where((path) => path.isNotEmpty);
    setState(() => _attachmentPaths.addAll(paths));
  }

  void _openAttachmentMenu() {
    _haptic();
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(18, 0, 18, 20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Add to chat',
                style: Theme.of(context).textTheme.titleLarge,
              ),
              const SizedBox(height: 12),
              _AttachTile(
                icon: Icons.camera_alt_outlined,
                title: 'Camera',
                subtitle: 'Capture an image for OpenMindAI Lens',
                onTap: () {
                  Navigator.pop(context);
                  _pickCamera();
                },
              ),
              _AttachTile(
                icon: Icons.photo_library_outlined,
                title: 'Photos',
                subtitle: 'Choose one or more images',
                onTap: () {
                  Navigator.pop(context);
                  _pickPhotos();
                },
              ),
              _AttachTile(
                icon: Icons.attach_file_rounded,
                title: 'Files',
                subtitle: 'PDF, DOCX, text, code, JSON, Markdown, YAML and CSV',
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
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 24),
          children: [
            OpenMindPageHeader(
              title: 'Choose model',
              subtitle:
                  'Select the local model used for new replies and Canvas.',
              trailing: IconButton(
                tooltip: 'Manage models',
                onPressed: () {
                  Navigator.pop(context);
                  _openModels();
                },
                icon: const Icon(Icons.download_rounded),
              ),
            ),
            const SizedBox(height: 14),
            ...MobileModelCatalog.models.map(
              (model) => Padding(
                padding: const EdgeInsets.only(bottom: 9),
                child: Card(
                  color: model.id == _selectedModelId
                      ? AppTheme.accent.withValues(alpha: .10)
                      : null,
                  child: ListTile(
                    leading: OpenMindFeatureIcon(
                      model.kind == 'Vision'
                          ? Icons.visibility_outlined
                          : model.kind == 'Reasoning'
                          ? Icons.psychology_outlined
                          : Icons.chat_bubble_outline_rounded,
                    ),
                    title: Text(
                      model.name,
                      style: const TextStyle(fontWeight: FontWeight.w700),
                    ),
                    subtitle: Text(
                      '${model.kind} · ${model.minRamGb}+ GB RAM · ~${model.sizeGb.toStringAsFixed(1)} GB',
                    ),
                    trailing: model.id == _selectedModelId
                        ? const Icon(
                            Icons.check_circle_rounded,
                            color: AppTheme.accent,
                          )
                        : const Icon(Icons.chevron_right_rounded),
                    onTap: () => Navigator.pop(context, model.id),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
    if (selected == null || !mounted) return;
    _haptic();
    await _stopSpeech();
    final model = MobileModelCatalog.byId(selected);
    if (!await _modelStorage.isInstalled(model)) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text('${model.name} is not installed yet.'),
          action: SnackBarAction(label: 'Install', onPressed: _openModels),
        ),
      );
      return;
    }
    if (selected != _selectedModelId) await _inference.shutdown();
    await _onboardingStore.setSelectedModelId(selected);
    if (mounted) setState(() => _selectedModelId = selected);
  }

  Future<void> _openModels() async {
    if (!mounted) return;
    _haptic();
    await showModelManagerSheet(
      context,
      storage: _modelStorage,
      onModelReady: (modelId) async {
        await _stopSpeech();
        if (modelId != _selectedModelId) await _inference.shutdown();
        await _onboardingStore.setSelectedModelId(modelId);
        if (mounted) setState(() => _selectedModelId = modelId);
      },
    );
  }

  Future<void> _openConversation(ChatConversation conversation) async {
    _haptic();
    await _stopSpeech();
    if (!mounted) return;
    setState(() => _activeConversationId = conversation.id);
    Navigator.pop(context);
    _scrollToBottom();
  }

  Future<void> _renameConversation(ChatConversation conversation) async {
    final controller = TextEditingController(text: conversation.title);
    final value = await showDialog<String>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Rename chat'),
        content: TextField(
          controller: controller,
          autofocus: true,
          maxLength: 80,
          decoration: const InputDecoration(hintText: 'Chat name'),
          onSubmitted: (text) => Navigator.pop(dialogContext, text),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(dialogContext, controller.text),
            child: const Text('Save'),
          ),
        ],
      ),
    );
    controller.dispose();
    final title = value?.trim();
    if (title == null || title.isEmpty) return;
    _haptic();
    conversation.title = title;
    conversation.updatedAt = DateTime.now();
    _conversations.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
    await _chatStore.save(_conversations);
    if (mounted) setState(() {});
  }

  Future<void> _deleteConversation(ChatConversation conversation) async {
    final approved = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Delete chat?'),
        content: Text(
          'Delete “${conversation.title}” and its saved attachments from this device?',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(dialogContext, true),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (approved != true) return;

    _haptic(strong: true);
    if (_activeConversationId == conversation.id && _generating) {
      await _stopGeneration();
    }
    final attachmentPaths = <String>{
      for (final message in conversation.messages) ...message.attachmentPaths,
    };
    _conversations.removeWhere((item) => item.id == conversation.id);
    if (_activeConversationId == conversation.id) {
      _activeConversationId = _conversations.isEmpty
          ? null
          : _conversations.first.id;
    }
    await _chatStore.save(_conversations);
    await _attachmentStorage.deletePaths(attachmentPaths);
    if (mounted) setState(() {});
  }

  Future<void> _openCanvas() async {
    _haptic();
    await _stopVoiceIfNeeded();
    await _stopSpeech();
    if (!mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            CanvasScreen(inference: _inference, modelId: _selectedModelId),
      ),
    );
  }

  Future<void> _openSettings() async {
    _haptic();
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => SettingsScreen(onHistoryChanged: _reloadHistory),
      ),
    );
    if (mounted) setState(() {});
  }

  void _useStarter(String prompt, String mode) {
    _haptic();
    _composer.text = prompt;
    _composer.selection = TextSelection.collapsed(offset: prompt.length);
    setState(() => _mode = mode);
  }

  void _changeMode(String value) {
    _haptic();
    setState(() => _mode = value);
  }

  @override
  Widget build(BuildContext context) {
    final conversation = _activeConversation;
    return Scaffold(
      key: _scaffoldKey,
      drawer: _buildDrawer(context),
      appBar: AppBar(
        leading: IconButton(
          tooltip: 'Open navigation',
          icon: const Icon(Icons.menu_rounded),
          onPressed: () => _scaffoldKey.currentState?.openDrawer(),
        ),
        titleSpacing: 2,
        title: InkWell(
          borderRadius: BorderRadius.circular(14),
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
                    style: const TextStyle(
                      fontSize: 17,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
                const SizedBox(width: 4),
                const Icon(Icons.keyboard_arrow_down_rounded, size: 20),
              ],
            ),
          ),
        ),
        actions: [
          IconButton(
            onPressed: _openCanvas,
            icon: const Icon(Icons.auto_awesome_rounded),
            tooltip: 'OpenMindAI Canvas',
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
                if (_mode != 'chat') _ModeBanner(mode: _mode),
                Expanded(
                  child: conversation == null || conversation.messages.isEmpty
                      ? _EmptyChat(
                          modelName: _selectedModel.name,
                          onModels: _openModels,
                          onStarter: _useStarter,
                        )
                      : ListView.builder(
                          controller: _scrollController,
                          padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
                          itemCount: conversation.messages.length,
                          itemBuilder: (context, index) {
                            final message = conversation.messages[index];
                            final lastAssistant =
                                message.role == 'assistant' &&
                                index == conversation.messages.length - 1;
                            return ChatMessageBubble(
                              message: message,
                              speaking: _speakingMessageId == message.id,
                              onSpeak: () => _toggleSpeech(message),
                              onRegenerate: lastAssistant && !_generating
                                  ? _regenerate
                                  : null,
                            );
                          },
                        ),
                ),
                ChatComposer(
                  controller: _composer,
                  mode: _mode,
                  attachmentPaths: _attachmentPaths,
                  generating: _generating,
                  voiceListening: _voiceListening,
                  voicePreparing: _voicePreparing,
                  onModeChanged: _changeMode,
                  onAdd: _openAttachmentMenu,
                  onVoice: _toggleVoice,
                  onSend: _send,
                  onStop: _stopGeneration,
                  onRemoveAttachment: (path) {
                    _haptic();
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
      width: MediaQuery.sizeOf(context).width.clamp(300, 360).toDouble(),
      child: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 10, 12, 8),
              child: Row(
                children: [
                  const OpenMindBrandMark(size: 38, compact: true),
                  const SizedBox(width: 11),
                  const Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'OpenMindAI',
                          style: TextStyle(
                            fontSize: 19,
                            fontWeight: FontWeight.w800,
                          ),
                        ),
                        Text(
                          'Local-first mobile AI',
                          style: TextStyle(fontSize: 12),
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    tooltip: 'New chat',
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
              padding: const EdgeInsets.fromLTRB(12, 6, 12, 4),
              child: Column(
                children: [
                  _DrawerAction(
                    icon: Icons.add_comment_outlined,
                    label: 'New chat',
                    onTap: () {
                      Navigator.pop(context);
                      _newChat();
                    },
                  ),
                  _DrawerAction(
                    icon: Icons.memory_rounded,
                    label: 'Models',
                    onTap: () {
                      Navigator.pop(context);
                      _openModels();
                    },
                  ),
                  _DrawerAction(
                    icon: Icons.auto_awesome_rounded,
                    label: 'OpenMindAI Canvas',
                    accent: true,
                    onTap: () {
                      Navigator.pop(context);
                      _openCanvas();
                    },
                  ),
                  _DrawerAction(
                    icon: Icons.settings_outlined,
                    label: 'Settings',
                    onTap: () {
                      Navigator.pop(context);
                      _openSettings();
                    },
                  ),
                ],
              ),
            ),
            const Padding(
              padding: EdgeInsets.symmetric(horizontal: 14, vertical: 4),
              child: Divider(),
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
            Padding(
              padding: const EdgeInsets.fromLTRB(18, 14, 18, 7),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(
                  'RECENT CHATS',
                  style: Theme.of(context).textTheme.labelSmall?.copyWith(
                    letterSpacing: 1.0,
                    fontWeight: FontWeight.w800,
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                ),
              ),
            ),
            Expanded(
              child: visibleConversations.isEmpty
                  ? Center(
                      child: Text(
                        _chatSearchQuery.isEmpty
                            ? 'No conversations yet'
                            : 'No matching chats',
                        style: TextStyle(
                          color: Theme.of(context).colorScheme.onSurfaceVariant,
                        ),
                      ),
                    )
                  : ListView.builder(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      itemCount: visibleConversations.length,
                      itemBuilder: (context, index) {
                        final item = visibleConversations[index];
                        return Padding(
                          padding: const EdgeInsets.only(bottom: 2),
                          child: ListTile(
                            dense: true,
                            selected: item.id == _activeConversationId,
                            selectedTileColor: AppTheme.accent.withValues(
                              alpha: .10,
                            ),
                            leading: const Icon(
                              Icons.chat_bubble_outline_rounded,
                              size: 18,
                            ),
                            title: Text(
                              item.title,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                            trailing: PopupMenuButton<String>(
                              tooltip: 'Chat actions',
                              onSelected: (value) {
                                if (value == 'rename') {
                                  _renameConversation(item);
                                } else if (value == 'delete') {
                                  _deleteConversation(item);
                                }
                              },
                              itemBuilder: (_) => const [
                                PopupMenuItem(
                                  value: 'rename',
                                  child: ListTile(
                                    contentPadding: EdgeInsets.zero,
                                    leading: Icon(Icons.edit_outlined),
                                    title: Text('Rename'),
                                  ),
                                ),
                                PopupMenuItem(
                                  value: 'delete',
                                  child: ListTile(
                                    contentPadding: EdgeInsets.zero,
                                    leading: Icon(Icons.delete_outline_rounded),
                                    title: Text('Delete'),
                                  ),
                                ),
                              ],
                            ),
                            onTap: () => _openConversation(item),
                          ),
                        );
                      },
                    ),
            ),
            const Divider(height: 1),
            const Padding(
              padding: EdgeInsets.fromLTRB(12, 8, 12, 12),
              child: ListTile(
                leading: CircleAvatar(child: Icon(Icons.lock_outline_rounded)),
                title: Text(
                  'Private by design',
                  style: TextStyle(fontWeight: FontWeight.w700),
                ),
                subtitle: Text('Core chat stays on this device'),
              ),
            ),
            const OpenMindAppFooter(),
          ],
        ),
      ),
    );
  }
}

class _ModeBanner extends StatelessWidget {
  const _ModeBanner({required this.mode});

  final String mode;

  @override
  Widget build(BuildContext context) {
    final (icon, title, detail) = switch (mode) {
      'thinking' => (
        Icons.psychology_outlined,
        'Think mode',
        'More deliberate local reasoning before the response.',
      ),
      'web-search' => (
        Icons.travel_explore_rounded,
        'Search mode',
        'Uses public web evidence when the current answer needs fresh information.',
      ),
      'research' => (
        Icons.biotech_outlined,
        'Research mode',
        'Collects and synthesizes multiple sources for a deeper answer.',
      ),
      _ => (Icons.chat_bubble_outline_rounded, 'Chat', 'General conversation.'),
    };
    return Container(
      width: double.infinity,
      margin: const EdgeInsets.fromLTRB(14, 7, 14, 0),
      padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 10),
      decoration: BoxDecoration(
        color: AppTheme.accent.withValues(alpha: .08),
        border: Border.all(color: AppTheme.accent.withValues(alpha: .20)),
        borderRadius: BorderRadius.circular(15),
      ),
      child: Row(
        children: [
          Icon(icon, size: 19, color: AppTheme.accent),
          const SizedBox(width: 9),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: const TextStyle(fontWeight: FontWeight.w800),
                ),
                Text(
                  detail,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _AttachTile extends StatelessWidget {
  const _AttachTile({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Card(
        child: ListTile(
          leading: OpenMindFeatureIcon(icon),
          title: Text(
            title,
            style: const TextStyle(fontWeight: FontWeight.w700),
          ),
          subtitle: Text(subtitle),
          trailing: const Icon(Icons.chevron_right_rounded),
          onTap: onTap,
        ),
      ),
    );
  }
}

class _DrawerAction extends StatelessWidget {
  const _DrawerAction({
    required this.icon,
    required this.label,
    required this.onTap,
    this.accent = false,
  });

  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final bool accent;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      dense: true,
      leading: Icon(icon, color: accent ? AppTheme.accent : null),
      title: Text(
        label,
        style: TextStyle(
          fontWeight: FontWeight.w700,
          color: accent ? AppTheme.accent : null,
        ),
      ),
      onTap: onTap,
    );
  }
}

class _EmptyChat extends StatelessWidget {
  const _EmptyChat({
    required this.modelName,
    required this.onModels,
    required this.onStarter,
  });

  final String modelName;
  final VoidCallback onModels;
  final void Function(String prompt, String mode) onStarter;

  @override
  Widget build(BuildContext context) {
    final starters = [
      (
        Icons.lightbulb_outline_rounded,
        'Explain something',
        'Explain quantum computing in simple terms.',
        'chat',
      ),
      (
        Icons.code_rounded,
        'Build with code',
        'Write a clean Flutter widget for a responsive profile card.',
        'thinking',
      ),
      (
        Icons.travel_explore_rounded,
        'Search the web',
        'Find the latest important AI platform updates.',
        'web-search',
      ),
      (
        Icons.biotech_outlined,
        'Research deeply',
        'Research current approaches to efficient on-device AI inference.',
        'research',
      ),
    ];

    return LayoutBuilder(
      builder: (context, constraints) {
        final maxWidth = constraints.maxWidth > 650
            ? 620.0
            : constraints.maxWidth;
        return Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(24, 28, 24, 28),
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: maxWidth),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const OpenMindBrandMark(size: 64),
                  const SizedBox(height: 20),
                  Text(
                    'How can I help?',
                    textAlign: TextAlign.center,
                    style: Theme.of(context).textTheme.headlineMedium,
                  ),
                  const SizedBox(height: 7),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      const Icon(
                        Icons.offline_bolt_outlined,
                        size: 15,
                        color: AppTheme.accent,
                      ),
                      const SizedBox(width: 5),
                      Flexible(
                        child: Text(
                          '$modelName · Local-first',
                          overflow: TextOverflow.ellipsis,
                          style: Theme.of(context).textTheme.bodySmall
                              ?.copyWith(
                                color: Theme.of(
                                  context,
                                ).colorScheme.onSurfaceVariant,
                              ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 24),
                  GridView.builder(
                    shrinkWrap: true,
                    physics: const NeverScrollableScrollPhysics(),
                    gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                      crossAxisCount: constraints.maxWidth > 520 ? 2 : 1,
                      childAspectRatio: constraints.maxWidth > 520 ? 2.7 : 4.1,
                      crossAxisSpacing: 10,
                      mainAxisSpacing: 10,
                    ),
                    itemCount: starters.length,
                    itemBuilder: (context, index) {
                      final item = starters[index];
                      return InkWell(
                        borderRadius: BorderRadius.circular(18),
                        onTap: () => onStarter(item.$3, item.$4),
                        child: Card(
                          child: Padding(
                            padding: const EdgeInsets.all(13),
                            child: Row(
                              children: [
                                OpenMindFeatureIcon(item.$1, size: 38),
                                const SizedBox(width: 11),
                                Expanded(
                                  child: Column(
                                    mainAxisAlignment: MainAxisAlignment.center,
                                    crossAxisAlignment:
                                        CrossAxisAlignment.start,
                                    children: [
                                      Text(
                                        item.$2,
                                        style: const TextStyle(
                                          fontWeight: FontWeight.w800,
                                        ),
                                      ),
                                      const SizedBox(height: 2),
                                      Text(
                                        item.$3,
                                        maxLines: 2,
                                        overflow: TextOverflow.ellipsis,
                                        style: Theme.of(
                                          context,
                                        ).textTheme.bodySmall,
                                      ),
                                    ],
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                  const SizedBox(height: 15),
                  TextButton.icon(
                    onPressed: onModels,
                    icon: const Icon(Icons.memory_rounded),
                    label: const Text('Manage local models'),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

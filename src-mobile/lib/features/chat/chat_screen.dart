import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:image_picker/image_picker.dart';
import '../../core/constants/model_catalog.dart';
import '../../core/storage/onboarding_store.dart';
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
  final _inference = NativeMobileInferenceService();
  final _imagePicker = ImagePicker();

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
    _load();
  }

  @override
  void dispose() {
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

  void _newChat() {
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
    final userText = text.isEmpty ? 'Review the attached file.' : text;
    final userMessage = ChatMessage(
      id: _id('message'),
      role: 'user',
      text: userText,
      createdAt: DateTime.now(),
    );

    final attachments = List<String>.from(_attachmentPaths);
    setState(() {
      conversation.messages.add(userMessage);
      conversation.updatedAt = DateTime.now();
      if (conversation.title == 'New chat') {
        conversation.title = userText.length > 42 ? '${userText.substring(0, 42)}…' : userText;
      }
      _composer.clear();
      _attachmentPaths.clear();
      _generating = true;
      _conversations.sort((a, b) => b.updatedAt.compareTo(a.updatedAt));
    });
    await _chatStore.save(_conversations);

    try {
      final response = await _inference.generate(
        MobileInferenceRequest(
          modelId: _selectedModelId,
          mode: _mode,
          messages: List<ChatMessage>.from(conversation.messages),
          attachmentPaths: attachments,
        ),
      );
      if (!mounted) return;
      setState(() {
        conversation.messages.add(ChatMessage(
          id: _id('message'),
          role: 'assistant',
          text: response,
          createdAt: DateTime.now(),
        ));
        conversation.updatedAt = DateTime.now();
      });
      await _chatStore.save(_conversations);
    } catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(error.toString()), behavior: SnackBarBehavior.floating),
      );
    } finally {
      if (mounted) setState(() => _generating = false);
    }
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
                subtitle: const Text('PDF, documents, code, text, and other supported files'),
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
            const Padding(
              padding: EdgeInsets.fromLTRB(12, 4, 12, 12),
              child: Text('Choose model', style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700)),
            ),
            ...MobileModelCatalog.models.map((model) => ListTile(
                  title: Text(model.name, style: const TextStyle(fontWeight: FontWeight.w600)),
                  subtitle: Text('${model.kind} · ${model.minRamGb}+ GB RAM'),
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
                      ? _EmptyChat(modelName: _selectedModel.name)
                      : ListView.builder(
                          padding: const EdgeInsets.fromLTRB(16, 18, 16, 24),
                          itemCount: conversation.messages.length + (_generating ? 1 : 0),
                          itemBuilder: (context, index) {
                            if (_generating && index == conversation.messages.length) {
                              return const _ThinkingRow();
                            }
                            return _MessageBubble(message: conversation.messages[index]);
                          },
                        ),
                ),
                _Composer(
                  controller: _composer,
                  mode: _mode,
                  attachmentCount: _attachmentPaths.length,
                  generating: _generating,
                  onModeChanged: (value) => setState(() => _mode = value),
                  onAdd: _openAttachmentMenu,
                  onSend: _send,
                  onClearAttachments: () => setState(_attachmentPaths.clear),
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
                readOnly: true,
                onTap: () {},
                decoration: const InputDecoration(prefixIcon: Icon(Icons.search_rounded), hintText: 'Search chats', isDense: true),
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
  const _EmptyChat({required this.modelName});
  final String modelName;

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
        ]),
      ),
    );
  }
}

class _MessageBubble extends StatelessWidget {
  const _MessageBubble({required this.message});
  final ChatMessage message;

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
        child: SelectableText(message.text, style: const TextStyle(fontSize: 16, height: 1.45)),
      ),
    );
  }
}

class _ThinkingRow extends StatelessWidget {
  const _ThinkingRow();

  @override
  Widget build(BuildContext context) {
    return const Padding(
      padding: EdgeInsets.only(bottom: 18),
      child: Row(children: [
        SizedBox(width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2)),
        SizedBox(width: 10),
        Text('Thinking…'),
      ]),
    );
  }
}

class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller,
    required this.mode,
    required this.attachmentCount,
    required this.generating,
    required this.onModeChanged,
    required this.onAdd,
    required this.onSend,
    required this.onClearAttachments,
  });

  final TextEditingController controller;
  final String mode;
  final int attachmentCount;
  final bool generating;
  final ValueChanged<String> onModeChanged;
  final VoidCallback onAdd;
  final VoidCallback onSend;
  final VoidCallback onClearAttachments;

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
          if (attachmentCount > 0)
            Padding(
              padding: const EdgeInsets.only(bottom: 6),
              child: Row(children: [
                Chip(
                  avatar: const Icon(Icons.attach_file_rounded, size: 17),
                  label: Text('$attachmentCount attachment${attachmentCount == 1 ? '' : 's'}'),
                  deleteIcon: const Icon(Icons.close_rounded, size: 17),
                  onDeleted: onClearAttachments,
                ),
              ]),
            ),
          TextField(
            controller: controller,
            minLines: 1,
            maxLines: 6,
            textInputAction: TextInputAction.newline,
            decoration: InputDecoration(
              hintText: 'Message OpenMindAI',
              contentPadding: const EdgeInsets.symmetric(vertical: 11),
              prefixIcon: IconButton(onPressed: onAdd, icon: const Icon(Icons.add_rounded)),
              suffixIcon: Padding(
                padding: const EdgeInsets.all(6),
                child: IconButton.filled(
                  onPressed: generating ? null : onSend,
                  icon: generating
                      ? const SizedBox(width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2))
                      : const Icon(Icons.arrow_upward_rounded),
                ),
              ),
            ),
            onSubmitted: (_) => onSend(),
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

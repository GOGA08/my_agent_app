import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:my_agent_app/src/rust/api/zeroclaw.dart' as zeroclaw;
import 'package:my_agent_app/src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(const ZeroClawApp());
}

class ZeroClawApp extends StatelessWidget {
  const ZeroClawApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'ZeroClaw Agent',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: Colors.deepPurple,
          brightness: Brightness.dark,
        ),
        useMaterial3: true,
      ),
      home: const ChatScreen(),
    );
  }
}

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final List<Map<String, String>> _messages = [];
  final TextEditingController _textController = TextEditingController();
  final TextEditingController _apiKeyController = TextEditingController();

  bool _isInit = false;
  bool _isLoading = false;
  String _statusMessage = "Starting...";

  @override
  void initState() {
    super.initState();
    _initAgent();
  }

  Future<void> _initAgent() async {
    try {
      final dir = await getApplicationDocumentsDirectory();
      
      // 1. Initialize the ZeroClaw Rust agent with local SQLite memory
      final status = await zeroclaw.initAgent(
        dataDir: dir.path, 
        strategy: "fallback"
      );
      
      setState(() {
        _isInit = true;
        _statusMessage = "Agent Ready!\nMemory Backend: ${status.memoryBackend}\n"
                         "Files: ${status.dataDir}";
      });
    } catch (e) {
      setState(() {
        _statusMessage = "Init Error: $e";
      });
    }
  }

  Future<void> _configureProvider() async {
    showDialog(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text("Set API Key"),
        content: TextField(
          controller: _apiKeyController,
          decoration: const InputDecoration(
            hintText: "Enter an API Key (Groq, Gemini, OpenAI)",
          ),
          obscureText: true,
        ),
        actions: [
          TextButton(
            onPressed: () async {
              Navigator.pop(context);
              
              if (_apiKeyController.text.isNotEmpty) {
                String key = _apiKeyController.text.trim();
                
                // Auto-detect provider based on key format
                String name = "gemini";
                String model = "gemini-2.5-flash";
                String baseUrl = "https://generativelanguage.googleapis.com/v1beta/openai";
                
                if (key.startsWith("gsk_")) {
                  name = "groq";
                  model = "llama-3.3-70b-versatile"; 
                  baseUrl = "https://api.groq.com/openai/v1";
                } else if (key.startsWith("sk-proj-") || key.startsWith("sk-")) {
                  name = "openai";
                  model = "gpt-4o-mini";
                  baseUrl = "https://api.openai.com/v1";
                }

                // 2. Add an LLM provider to the Rust registry
                await zeroclaw.addProvider(
                  name: name,
                  apiKey: key,
                  model: model, 
                  baseUrl: baseUrl,
                  priority: 1,
                );
                
                ScaffoldMessenger.of(context).showSnackBar(
                  SnackBar(content: Text('Configured $name ($model)!')),
                );
              }
            },
            child: const Text("Save"),
          ),
        ],
      ),
    );
  }

  Future<void> _handleSubmitted(String text) async {
    if (text.trim().isEmpty || !_isInit) return;

    _textController.clear();
    setState(() {
      _messages.insert(0, {"role": "user", "text": text});
      _isLoading = true;
    });

    try {
      // 3. Send the prompt to ZeroClaw (Auto-Recall + Routes to Provider + Stores output)
      // Note: Because we used placeholder HTTP code in Rust, it will just echo back
      // unless you replace `call_provider()` in Rust with an actual HTTP client!
      final reply = await zeroclaw.runAgent(
        prompt: text, 
        sessionId: "default",
      );

      setState(() {
        _messages.insert(0, {"role": "agent", "text": reply});
      });
    } catch (e) {
      setState(() {
        _messages.insert(0, {"role": "agent", "text": "Error: $e\n\nDid you add an API key first?"});
      });
    } finally {
      setState(() {
        _isLoading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!_isInit) {
      return Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const CircularProgressIndicator(),
              const SizedBox(height: 16),
              Text(_statusMessage, textAlign: TextAlign.center),
            ],
          ),
        ),
      );
    }

    return Scaffold(
      appBar: AppBar(
        title: const Text('ZeroClaw Chat'),
        actions: [
          IconButton(
            icon: const Icon(Icons.key),
            onPressed: _configureProvider,
            tooltip: "Set API Key",
          ),
        ],
      ),
      body: Column(
        children: [
          // Chat list
          Expanded(
            child: ListView.builder(
              reverse: true,
              padding: const EdgeInsets.all(8.0),
              itemCount: _messages.length,
              itemBuilder: (_, int index) {
                final msg = _messages[index];
                final isUser = msg["role"] == "user";
                return Align(
                  alignment: isUser ? Alignment.centerRight : Alignment.centerLeft,
                  child: Container(
                    margin: const EdgeInsets.symmetric(vertical: 4.0),
                    padding: const EdgeInsets.all(12.0),
                    decoration: BoxDecoration(
                      color: isUser ? Colors.deepPurple[400] : Colors.grey[800],
                      borderRadius: BorderRadius.circular(16.0),
                    ),
                    constraints: BoxConstraints(
                      maxWidth: MediaQuery.of(context).size.width * 0.8,
                    ),
                    child: Text(
                      msg["text"]!,
                      style: const TextStyle(color: Colors.white),
                    ),
                  ),
                );
              },
            ),
          ),
          
          if (_isLoading)
            const Padding(
              padding: EdgeInsets.all(8.0),
              child: LinearProgressIndicator(),
            ),

          // Input field
          Container(
            decoration: BoxDecoration(color: Theme.of(context).cardColor),
            child: SafeArea(
              child: Row(
                children: [
                  const SizedBox(width: 8),
                  Expanded(
                    child: TextField(
                      controller: _textController,
                      decoration: const InputDecoration(
                        hintText: 'Talk to your agent...',
                        border: InputBorder.none,
                      ),
                      onSubmitted: _handleSubmitted,
                    ),
                  ),
                  IconButton(
                    icon: const Icon(Icons.send),
                    onPressed: () => _handleSubmitted(_textController.text),
                    color: Colors.deepPurpleAccent,
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

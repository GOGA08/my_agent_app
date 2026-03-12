# 🤖 ZeroClaw Mobile Agent

> **Private, Local-First, and Blazing Fast.**  
> A cross-platform AI Assistant built with Flutter and Rust, featuring on-device SQLite vector memory and multi-provider LLM routing.

[![Flutter](https://img.shields.io/badge/Flutter-02569B?style=for-the-badge&logo=flutter&logoColor=white)](https://flutter.dev)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![SQLite](https://img.shields.io/badge/SQLite-07405E?style=for-the-badge&logo=sqlite&logoColor=white)](https://www.sqlite.org/)

---

## ✨ Key Features

- **🧠 Local Memory (Long-Term)**: Uses ZeroClaw's Rust engine to manage an on-device SQLite vector database. Every chat is indexed locally for hybrid vector/keyword search.
- **🛡️ 100% Private**: Your memories and conversation history never leave your device.
- **🚀 Multi-Provider Routing**: Seamlessly switch between **Groq** (LLaMA 3), **OpenAI** (GPT-4), and **Google Gemini**.
- **🔄 Automatic Fallback**: If one provider goes down or you hit a rate limit, the agent automatically fails over to the next registered provider.
- **⚡ Native Rust Speed**: Core logic, vector embeddings, and networking are handled in highly-optimized Rust via `flutter_rust_bridge`.

## 🛠️ Tech Stack

- **Frontend**: Flutter (Dart)
- **Backend**: Rust (Agent Engine)
- **Memory**: ZeroClaw + SQLite
- **Network**: Reqwest + Rustls (Secure TLS)
- **Bridge**: `flutter_rust_bridge`

## 🚀 Getting Started

### Prerequisites

- [Flutter SDK](https://docs.flutter.dev/get-started/install)
- [Rust Toolchain](https://www.rust-lang.org/tools/install)
- [Cargo NDK](https://github.com/bbqsrc/cargo-ndk) (for Android builds)

### Build & Run

1. **Clone the repo**
   ```bash
   git clone https://github.com/YOUR_USERNAME/my_agent_app.git
   cd my_agent_app
   ```

2. **Run the app**
   ```bash
   flutter run
   ```

3. **Build Release APK**
   ```bash
   flutter build apk --release
   ```

## 🔑 Configuration

Open the app on your phone and tap the **Key Icon** 🔑 to add your API keys. ZeroClaw auto-detects the provider:
- `gsk_...` -> Groq (Llama 3)
- `sk-...` -> OpenAI (GPT-4o)
- `AIza...` -> Google Gemini

## 📜 License

MIT License - feel free to build and share!

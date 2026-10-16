# macOS Settings and Onboarding 1-click UI

Type: prototype
Status: resolved
Assignee: Antigravity
Blocked by:
- [In-app 1-click model download manager and storage lifecycle](./03-model-download-manager.md)

## Question

How should the Settings UI in `apps/macos/Sources/Chorus/Settings.swift` present the 1-click install card, download progress bar, storage size indicator, and model status, replacing the old "Ollama ในเครื่อง" UI?

Specifically design:
1. Native SwiftUI Card in `Settings.swift` under the Models/Router section:
   - State "Not Installed": Displays "Local Intelligence Model (~500 MB)" with a prominent "Download & Enable" button.
   - State "Downloading": Shows a live animated ProgressView with percentage, download speed, and estimated time remaining, plus a "Cancel" button.
   - State "Installed & Ready": Shows a green checkmark badge, model name, file size on disk (e.g. "980 MB in Application Support"), and a subtle "Delete Model" button to free up disk space.
2. Status pill in Composer / Router: Shows whether routing is performed by "Local SLM (Metal)" or "Fast In-Process Rule" fallback.
3. User experience guidelines adhering to Apple Human Interface Guidelines and `anti-ui-slop`.

## Answer

- **Zero-Ollama UI Design**: Replaced legacy "Ollama ในเครื่อง" rows in `apps/macos/Sources/Chorus/Settings.swift` with a modern, native Apple HIG card under "ระบบเลือกโมเดลอัตโนมัติ (Zero-Ollama Auto Router)".
- **Three-State Native Flow**:
  1. *Not Installed*: Displays "OpenThai System One 0.8B (แนะนำ)" with a prominent SF Symbol button `Label("ดาวน์โหลดคลิกเดียว (529 MB)", systemImage: "arrow.down.circle.fill")`.
  2. *Downloading*: Animated inline `ProgressView` bound to `engine.localModelDownloadProgress` with dynamic MB/percentage label and "ยกเลิก" cancel button.
  3. *Installed & Ready*: Green `checkmark.circle.fill` badge with exact file size on disk and a bordered "ลบโมเดล" button to reclaim storage.
- **Dual Model Support**: Added an optional secondary card for the AI Conversation Compactor (`qwen2.5-coder:1.5b-instruct-q3_k_m`) while noting that model-free Extractive (0 MB) remains active as the default.
- **Verification**: Built cleanly via `swift build` and integrated directly into the running macOS shell.

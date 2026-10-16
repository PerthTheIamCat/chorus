import SwiftUI
import Speech
import AVFoundation

/// Voice-to-Code Dictation Manager (Ticket 11 & Frontier Super App).
/// Provides offline / native on-device speech recognition via Apple Speech.framework.
/// Allows developers to dictate prompts hands-free in Thai (th-TH) or English (en-US).
public final class VoiceDictationManager: ObservableObject {
    public static let shared = VoiceDictationManager()

    @Published public var isRecording = false
    @Published public var transcribedText = ""
    @Published public var audioLevel: Float = 0.0
    @Published public var errorMessage: String?

    private var audioEngine = AVAudioEngine()
    private var speechRecognizer: SFSpeechRecognizer?
    private var recognitionRequest: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?

    public init() {}

    public func toggleRecording(language: String = "th-TH", onTranscript: @escaping (String) -> Void) {
        if isRecording {
            stopRecording()
        } else {
            startRecording(language: language, onTranscript: onTranscript)
        }
    }

    public func startRecording(language: String = "th-TH", onTranscript: @escaping (String) -> Void) {
        // Cancel existing task if any
        stopRecording()

        speechRecognizer = SFSpeechRecognizer(locale: Locale(identifier: language))
        guard let speechRecognizer, speechRecognizer.isAvailable else {
            self.errorMessage = "Speech Recognizer ไม่พร้อมใช้งานสำหรับภาษา \(language)"
            return
        }

        SFSpeechRecognizer.requestAuthorization { [weak self] authStatus in
            DispatchQueue.main.async {
                guard let self else { return }
                switch authStatus {
                case .authorized:
                    self.beginAudioCapture(onTranscript: onTranscript)
                case .denied:
                    self.errorMessage = "ไม่ได้รับอนุญาตให้ใช้การรู้จำเสียง (Speech Recognition Denied)"
                case .restricted:
                    self.errorMessage = "ระบบจำกัดการเข้าถึงการรู้จำเสียง (Restricted)"
                case .notDetermined:
                    self.errorMessage = "ยังไม่ได้ระบุสิทธิ์การใช้งานไมโครโฟน"
                @unknown default:
                    self.errorMessage = "เกิดข้อผิดพลาดในการขอสิทธิ์"
                }
            }
        }
    }

    private func beginAudioCapture(onTranscript: @escaping (String) -> Void) {
        let node = audioEngine.inputNode
        let recordingFormat = node.outputFormat(forBus: 0)

        recognitionRequest = SFSpeechAudioBufferRecognitionRequest()
        guard let recognitionRequest else { return }
        recognitionRequest.shouldReportPartialResults = true

        recognitionTask = speechRecognizer?.recognitionTask(with: recognitionRequest) { [weak self] result, error in
            guard let self else { return }
            if let result {
                let text = result.bestTranscription.formattedString
                DispatchQueue.main.async {
                    self.transcribedText = text
                    onTranscript(text)
                }
            }
            if error != nil || (result?.isFinal ?? false) {
                self.stopRecording()
            }
        }

        node.installTap(onBus: 0, bufferSize: 1024, format: recordingFormat) { [weak self] buffer, _ in
            self?.recognitionRequest?.append(buffer)

            // Calculate rough RMS audio power for waveform animation
            guard let channelData = buffer.floatChannelData?[0] else { return }
            let channelDataArray = Array(UnsafeBufferPointer(start: channelData, count: Int(buffer.frameLength)))
            var sum: Float = 0
            for sample in channelDataArray { sum += sample * sample }
            let rms = sqrt(sum / Float(buffer.frameLength))
            DispatchQueue.main.async {
                self?.audioLevel = min(max(rms * 10, 0.0), 1.0)
            }
        }

        audioEngine.prepare()
        do {
            try audioEngine.start()
            isRecording = true
            errorMessage = nil
        } catch {
            errorMessage = "เริ่มบันทึกเสียงไม่ได้: \(error.localizedDescription)"
            stopRecording()
        }
    }

    public func stopRecording() {
        if audioEngine.isRunning {
            audioEngine.stop()
            audioEngine.inputNode.removeTap(onBus: 0)
        }
        recognitionRequest?.endAudio()
        recognitionTask?.cancel()
        recognitionRequest = nil
        recognitionTask = nil
        DispatchQueue.main.async {
            self.isRecording = false
            self.audioLevel = 0.0
        }
    }
}

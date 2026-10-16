import Foundation
import AppKit
import Vision

/// Native on-device OCR powered by Apple Vision Framework.
///
/// Enables instant zero-cloud text extraction from screenshots, diagrams, and logs
/// pasted into Chorus or dropped as attachments, preventing unnecessary vision token costs
/// for models that perform better on clean markdown/text.
final class VisionOCR {
    static let shared = VisionOCR()

    private init() {}

    struct OCRResult {
        let text: String
        let lineCount: Int
        let averageConfidence: Float
    }

    /// Performs on-device text recognition on an NSImage.
    func recognizeText(from image: NSImage) async throws -> OCRResult {
        guard let tiffData = image.tiffRepresentation,
              let bitmapImage = NSBitmapImageRep(data: tiffData),
              let cgImage = bitmapImage.cgImage else {
            throw NSError(domain: "dev.chorus.ocr", code: 1, userInfo: [NSLocalizedDescriptionKey: "Failed to convert image to CGImage"])
        }

        return try await recognizeText(from: cgImage)
    }

    /// Performs on-device text recognition on raw image data.
    func recognizeText(from data: Data) async throws -> OCRResult {
        guard let nsImage = NSImage(data: data) else {
            throw NSError(domain: "dev.chorus.ocr", code: 2, userInfo: [NSLocalizedDescriptionKey: "Invalid image data"])
        }
        return try await recognizeText(from: nsImage)
    }

    /// Core Vision OCR routine executing on Apple Neural Engine / GPU.
    func recognizeText(from cgImage: CGImage) async throws -> OCRResult {
        return try await withCheckedThrowingContinuation { continuation in
            let request = VNRecognizeTextRequest { request, error in
                if let error = error {
                    continuation.resume(throwing: error)
                    return
                }

                guard let observations = request.results as? [VNRecognizedTextObservation] else {
                    continuation.resume(returning: OCRResult(text: "", lineCount: 0, averageConfidence: 1.0))
                    return
                }

                var lines: [String] = []
                var totalConfidence: Float = 0.0

                for observation in observations {
                    if let candidate = observation.topCandidates(1).first {
                        lines.append(candidate.string)
                        totalConfidence += candidate.confidence
                    }
                }

                let fullText = lines.joined(separator: "\n")
                let avgConfidence = lines.isEmpty ? 1.0 : (totalConfidence / Float(lines.count))

                continuation.resume(returning: OCRResult(
                    text: fullText,
                    lineCount: lines.count,
                    averageConfidence: avgConfidence
                ))
            }

            request.recognitionLevel = .accurate
            request.usesLanguageCorrection = true

            let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
            do {
                try handler.perform([request])
            } catch {
                continuation.resume(throwing: error)
            }
        }
    }
}

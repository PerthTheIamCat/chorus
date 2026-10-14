import SwiftUI
import CoreImage.CIFilterBuiltins
import CChorus

private struct PairingSessionData: Decodable {
    let daemon_id: String
    let machine_name: String
    let pairing_token: String
    let endpoint_url: String
    let tailscale_ip: String?
    let created_at: UInt64
    let expires_at: UInt64
}

private struct PairingInitiateResponse: Decodable {
    let ok: Bool
    let session: PairingSessionData?
    let error: String?
}

private struct DeviceMetadata: Decodable {
    let device_id: String
    let device_name: String
    let paired_at: UInt64
    let last_seen_at: UInt64
}

private struct PairingDevicesResponse: Decodable {
    let ok: Bool
    let devices: [DeviceMetadata]?
    let error: String?
}

/// Remote Mobile Companion Pairing View (Phase 6).
/// Displays a high-contrast QR code generated via CoreImage for instantaneous
/// pairing with iPhone / iPad companion apps over local network or Tailscale/WireGuard.
struct RemotePairingView: View {
    @Environment(\.dismiss) private var dismiss

    @State private var pairingToken: String = ""
    @State private var daemonEndpoint: String = "http://127.0.0.1:4040"
    @State private var qrImage: NSImage? = nil
    @State private var pairedDevices: [PairedDeviceItem] = []
    @State private var isLoading = true
    @State private var copied = false

    struct PairedDeviceItem: Identifiable {
        let id: String
        let name: String
        let pairedAt: String
        let lastSeen: String
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack {
                HStack(spacing: 8) {
                    Image(systemName: "iphone.and.arrow.forward")
                        .font(.title2)
                        .foregroundStyle(Color.accentColor)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Mobile Companion Pairing")
                            .font(.headline)
                        Text("เชื่อมต่อ iPhone หรือ iPad เพื่ออนุมัติคำสั่งและติดตามความคืบหน้าแบบ Realtime")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Button { dismiss() } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                        .font(.title3)
                }
                .buttonStyle(.plain)
            }
            .padding()
            .background(.regularMaterial)

            Divider()

            ScrollView {
                VStack(spacing: 24) {
                    // QR Code Card
                    VStack(spacing: 16) {
                        if let qr = qrImage {
                            Image(nsImage: qr)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 200, height: 200)
                                .padding(12)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                .shadow(color: .black.opacity(0.1), radius: 8, y: 4)
                        } else {
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .fill(Color.secondary.opacity(0.1))
                                .frame(width: 200, height: 200)
                                .overlay {
                                    ProgressView("กำลังสร้าง QR Code…")
                                }
                        }

                        VStack(spacing: 6) {
                            Text("สแกนด้วยกล้อง iPhone หรือเปิด Chorus iOS")
                                .font(.subheadline.weight(.semibold))
                            Text("รหัสจับคู่มีอายุ 5 นาที · ปลอดภัยด้วย Ed25519 Secret Handshake")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }

                        if !pairingToken.isEmpty {
                            HStack(spacing: 8) {
                                Text(pairingToken)
                                    .font(.system(.caption, design: .monospaced).weight(.medium))
                                    .textSelection(.enabled)
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))

                                Button {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(pairingToken, forType: .string)
                                    copied = true
                                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) { copied = false }
                                } label: {
                                    Label(copied ? "คัดลอกแล้ว" : "คัดลอกรหัส", systemImage: copied ? "checkmark" : "doc.on.doc")
                                        .font(.caption)
                                }
                                .buttonStyle(.bordered)
                            }
                        }
                    }
                    .padding(20)
                    .frame(maxWidth: .infinity)
                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Color.primary.opacity(0.08), lineWidth: 1))

                    // Paired Devices List
                    VStack(alignment: .leading, spacing: 12) {
                        Text("อุปกรณ์ที่จับคู่แล้ว (\(pairedDevices.count))")
                            .font(.headline)

                        if pairedDevices.isEmpty {
                            HStack {
                                Image(systemName: "shield.slash")
                                    .foregroundStyle(.secondary)
                                Text("ยังไม่มีอุปกรณ์เชื่อมต่อ — สแกน QR ด้านบนเพื่อจับคู่อุปกรณ์แรก")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            .padding()
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                        } else {
                            VStack(spacing: 8) {
                                ForEach(pairedDevices) { dev in
                                    HStack {
                                        Image(systemName: "iphone")
                                            .font(.title3)
                                            .foregroundStyle(Color.accentColor)
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(dev.name)
                                                .font(.subheadline.weight(.medium))
                                            Text("เชื่อมต่อเมื่อ \(dev.pairedAt) · ใช้งานล่าสุด \(dev.lastSeen)")
                                                .font(.caption2)
                                                .foregroundStyle(.secondary)
                                        }
                                        Spacer()
                                        Button(role: .destructive) {
                                            revokeDevice(id: dev.id)
                                        } label: {
                                            Text("ยกเลิกการเชื่อมต่อ")
                                                .font(.caption)
                                        }
                                        .buttonStyle(.bordered)
                                    }
                                    .padding(10)
                                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 8))
                                }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding()
            }
        }
        .frame(width: 480, height: 560)
        .onAppear {
            generatePairingSession()
        }
    }

    private func generatePairingSession() {
        isLoading = true
        DispatchQueue.global(qos: .userInitiated).async {
            guard let ptr = chorus_pairing_initiate_json() else {
                DispatchQueue.main.async { self.isLoading = false }
                return
            }
            defer { chorus_string_free(ptr) }
            let jsonString = String(cString: ptr)
            if let data = jsonString.data(using: .utf8),
               let resp = try? JSONDecoder().decode(PairingInitiateResponse.self, from: data),
               resp.ok, let session = resp.session {
                let targetEndpoint = session.tailscale_ip != nil ? "http://\(session.tailscale_ip!):24678" : session.endpoint_url
                let payload = "chorus://pair?token=\(session.pairing_token)&endpoint=\(targetEndpoint)"
                let qr = generateQRCode(from: payload)
                DispatchQueue.main.async {
                    self.pairingToken = session.pairing_token
                    self.daemonEndpoint = targetEndpoint
                    self.qrImage = qr
                    self.isLoading = false
                    self.fetchDevices()
                }
            } else {
                DispatchQueue.main.async {
                    self.isLoading = false
                }
            }
        }
    }

    private func fetchDevices() {
        DispatchQueue.global(qos: .userInitiated).async {
            guard let ptr = chorus_pairing_devices_json() else { return }
            defer { chorus_string_free(ptr) }
            let jsonString = String(cString: ptr)
            if let data = jsonString.data(using: .utf8),
               let resp = try? JSONDecoder().decode(PairingDevicesResponse.self, from: data),
               resp.ok, let devs = resp.devices {
                let items = devs.map { d in
                    let pairedDate = Date(timeIntervalSince1970: TimeInterval(d.paired_at))
                    let lastSeenDate = Date(timeIntervalSince1970: TimeInterval(d.last_seen_at))
                    let formatter = RelativeDateTimeFormatter()
                    formatter.unitsStyle = .short
                    return PairedDeviceItem(
                        id: d.device_id,
                        name: d.device_name,
                        pairedAt: formatter.localizedString(for: pairedDate, relativeTo: Date()),
                        lastSeen: formatter.localizedString(for: lastSeenDate, relativeTo: Date())
                    )
                }
                DispatchQueue.main.async {
                    self.pairedDevices = items
                }
            }
        }
    }

    private func generateQRCode(from string: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(Data(string.utf8), forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")

        guard let ciImage = filter.outputImage else { return nil }

        // Scale up to crisp resolution
        let transform = CGAffineTransform(scaleX: 10, y: 10)
        let scaledCIImage = ciImage.transformed(by: transform)

        let rep = NSCIImageRep(ciImage: scaledCIImage)
        let nsImage = NSImage(size: rep.size)
        nsImage.addRepresentation(rep)
        return nsImage
    }

    private func revokeDevice(id: String) {
        DispatchQueue.global(qos: .userInitiated).async {
            if let ptr = chorus_pairing_revoke_json(id) {
                chorus_string_free(ptr)
            }
            DispatchQueue.main.async {
                self.pairedDevices.removeAll { $0.id == id }
            }
        }
    }
}

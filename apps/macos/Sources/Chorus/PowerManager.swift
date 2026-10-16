import Foundation
import Combine

/// Manages macOS Power, App Nap prevention, and Apple Silicon Thermal States.
///
/// Under long-running agent workflows (e.g. multi-step code generation or verification),
/// macOS may throttle background execution via App Nap or idle sleep.
/// `PowerManager` engages `ProcessInfo.beginActivity` during active turns to maintain
/// full Performance Core scheduling, and monitors thermal states to adapt concurrency.
@MainActor
final class PowerManager: ObservableObject {
    static let shared = PowerManager()

    @Published private(set) var activeActivityCount: Int = 0
    @Published private(set) var currentThermalState: ProcessInfo.ThermalState = .nominal
    @Published private(set) var isLowPowerMode: Bool = false

    private var activityToken: NSObjectProtocol?
    private var thermalObserver: AnyCancellable?

    private init() {
        self.currentThermalState = ProcessInfo.processInfo.thermalState
        self.isLowPowerMode = ProcessInfo.processInfo.isLowPowerModeEnabled

        NotificationCenter.default.addObserver(
            forName: ProcessInfo.thermalStateDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.updateThermalState()
            }
        }

        NotificationCenter.default.addObserver(
            forName: NSNotification.Name.NSProcessInfoPowerStateDidChange,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.isLowPowerMode = ProcessInfo.processInfo.isLowPowerModeEnabled
            }
        }
    }

    private func updateThermalState() {
        self.currentThermalState = ProcessInfo.processInfo.thermalState
    }

    /// Thermal pressure description for status UI and telemetry.
    var thermalStateName: String {
        switch currentThermalState {
        case .nominal: return "Nominal (Cool)"
        case .fair: return "Fair (Warm)"
        case .serious: return "Serious (Throttling)"
        case .critical: return "Critical (Severe Throttle)"
        @unknown default: return "Unknown"
        }
    }

    /// Whether thermal conditions or low power mode require throttling agent concurrency.
    var shouldThrottleConcurrency: Bool {
        return currentThermalState == .serious || currentThermalState == .critical || isLowPowerMode
    }

    /// Whether local model inference should fallback to 0ms fast rule classifier
    /// to preserve battery or reduce thermal load.
    var shouldConserveEnergyForInference: Bool {
        let conservePref = UserDefaults.standard.object(forKey: "chorus.conserveBatteryOnInference") as? Bool ?? true
        return (conservePref && isLowPowerMode) || currentThermalState == .serious || currentThermalState == .critical
    }

    /// Recommended max concurrent subagents based on Apple Silicon thermal budget.
    var recommendedMaxSubagents: Int {
        if isLowPowerMode { return 1 }
        switch currentThermalState {
        case .nominal: return 4
        case .fair: return 2
        case .serious: return 1
        case .critical: return 1
        @unknown default: return 2
        }
    }

    /// Begins a high-priority power activity preventing App Nap and idle sleep.
    func beginActivity(reason: String = "Active Agent Execution") {
        activeActivityCount += 1
        if activityToken == nil {
            activityToken = ProcessInfo.processInfo.beginActivity(
                options: [.userInitiated, .idleSystemSleepDisabled],
                reason: reason
            )
        }
    }

    /// Ends an activity assertion when an agent turn or background task completes.
    func endActivity() {
        guard activeActivityCount > 0 else { return }
        activeActivityCount -= 1
        if activeActivityCount == 0, let token = activityToken {
            ProcessInfo.processInfo.endActivity(token)
            activityToken = nil
        }
    }
}

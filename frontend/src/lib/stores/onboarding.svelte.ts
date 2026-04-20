// Onboarding wizard state. Tracks whether the GUI wizard needs to run,
// the current step, detected host config, and asset download status.

import * as api from '../api';
import type {
  SetupStateResponse,
  DetectedConfigSummary,
} from '../types/onboarding';

const TOTAL_STEPS = 4;
const ASSET_POLL_INTERVAL = 3000;

class OnboardingStore {
  needsOnboarding = $state(false);
  /** Whether `capsem setup` has finished. If false, the app should warn
   *  the user that the install never completed. */
  installCompleted = $state(true);
  /** True while a retry is in flight, so the banner button can disable. */
  retrying = $state(false);
  /** Last retry error message, surfaced inline in the banner. */
  retryError = $state<string | null>(null);
  loading = $state(true);
  currentStep = $state(0);
  totalSteps = TOTAL_STEPS;

  // Setup state from backend
  setupState = $state<SetupStateResponse | null>(null);

  // Host detection results
  detected = $state<DetectedConfigSummary | null>(null);
  detecting = $state(false);

  // Asset status (from GET /status -- the gateway endpoint)
  assetsReady = $state(false);
  assetsMissing = $state<string[]>([]);
  assetsVersion = $state<string | null>(null);

  #assetPollTimer: ReturnType<typeof setInterval> | null = null;

  /** Check if onboarding is needed. Called once from App.svelte after gateway connects. */
  async checkOnboarding(): Promise<void> {
    this.loading = true;
    try {
      const state = await api.getSetupState();
      this.setupState = state;
      // The server computes `needs_onboarding` using the current wizard
      // version, so we never have to mirror that constant on the client.
      this.needsOnboarding = state.needs_onboarding;
      this.installCompleted = state.install_completed;
    } catch {
      // If the endpoint doesn't exist (old service), skip onboarding and
      // assume install is complete -- nothing to warn about if we can't ask.
      this.needsOnboarding = false;
      this.installCompleted = true;
    } finally {
      this.loading = false;
    }
  }

  /** Run host detection (writes to settings, returns summary). */
  async runDetection(): Promise<void> {
    this.detecting = true;
    try {
      this.detected = await api.runDetection();
    } catch {
      // Detection failed -- leave detected as null
    } finally {
      this.detecting = false;
    }
  }

  /** Load asset status from the gateway's GET /status endpoint. */
  async loadAssetStatus(): Promise<void> {
    try {
      const status = await api.getStatus();
      if (status.assets) {
        this.assetsReady = status.assets.ready;
        this.assetsMissing = status.assets.missing;
        this.assetsVersion = status.assets.version ?? null;
      }
    } catch {
      // Status unavailable
    }
  }

  /** Start polling asset status at intervals. */
  startAssetPolling(): void {
    this.stopAssetPolling();
    this.#assetPollTimer = setInterval(() => {
      this.loadAssetStatus().then(() => {
        if (this.assetsReady) {
          this.stopAssetPolling();
        }
      });
    }, ASSET_POLL_INTERVAL);
  }

  /** Stop asset polling. */
  stopAssetPolling(): void {
    if (this.#assetPollTimer) {
      clearInterval(this.#assetPollTimer);
      this.#assetPollTimer = null;
    }
  }

  /** Retry `capsem setup` server-side. On success, refresh setup state so
   *  the banner disappears. On failure, store the error for display. */
  async retryInstall(): Promise<void> {
    if (this.retrying) return;
    this.retrying = true;
    this.retryError = null;
    try {
      await api.retrySetup();
      await this.checkOnboarding();
    } catch (e) {
      this.retryError = e instanceof Error ? e.message : String(e);
    } finally {
      this.retrying = false;
    }
  }

  /** Mark onboarding as complete and dismiss the wizard. */
  async completeOnboarding(): Promise<void> {
    try {
      await api.completeOnboarding();
    } catch {
      // Best-effort -- the wizard still dismisses
    }
    this.needsOnboarding = false;
    this.stopAssetPolling();
  }

  /** Navigate to a specific step. */
  goToStep(step: number): void {
    if (step >= 0 && step < this.totalSteps) {
      this.currentStep = step;
    }
  }

  /** Advance to the next step. */
  nextStep(): void {
    this.goToStep(this.currentStep + 1);
  }

  /** Go back one step. */
  prevStep(): void {
    this.goToStep(this.currentStep - 1);
  }

  destroy(): void {
    this.stopAssetPolling();
  }
}

export const onboardingStore = new OnboardingStore();

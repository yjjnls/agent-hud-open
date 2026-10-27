import { isTauri } from "@tauri-apps/api/core";

/**
 * WKWebView throttles animation frames for windows of an inactive app, and a
 * HUD's non-activating panel keeps the app inactive for its whole life: the
 * bots freeze at whatever frame they were drawn at. WebKit spares pages it
 * counts as playing audio from that throttling, so the HUD keeps a context
 * audible in WebKit's eyes: a sub-bass oscillator no speaker can reproduce.
 * A zero gain does not survive the audit — WebKit drops the assertion the
 * moment it decides nothing is actually sounding.
 */
export function startAnimationKeepalive(): void {
  if (!isTauri()) return;
  try {
    const context = new AudioContext();
    const oscillator = context.createOscillator();
    oscillator.frequency.value = 15;
    const gain = context.createGain();
    gain.gain.value = 0.01;
    oscillator.connect(gain).connect(context.destination);
    oscillator.start();
    const resume = () => {
      if (context.state !== "running") void context.resume().catch(() => {});
    };
    document.addEventListener("visibilitychange", resume);
    resume();
  } catch {
    // No audio context: the HUD then animates only while the app is active.
  }
}

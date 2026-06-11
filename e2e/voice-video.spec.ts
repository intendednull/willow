import { test, expect, Page } from '@playwright/test';
import { setupTwoPeers } from './helpers/peers';
import { visibleShell } from './helpers/ui';

// End-to-end proof that real WebRTC media flows between two peers.
//
// Runs ONLY under the `voice-chrome` Playwright project, which launches
// Chromium with `--use-fake-device-for-media-stream` /
// `--use-fake-ui-for-media-stream`: a synthetic mic + camera (no hardware) and
// auto-granted media permissions. Two browser contexts on this one machine
// then establish a *real* RTCPeerConnection over loopback host candidates — no
// STUN/TURN needed same-host — so this exercises the whole media path
// (signaling over iroh gossip → SDP exchange → ICE → tracks) end to end.
//
// Cross-NAT traversal (RC1) cannot be proven here (both contexts share the
// host network); that needs the manual 2-machine protocol in
// docs/reports/2026-06-07-voice-media-connectivity-investigation.md.
//
// What this guards:
//   * RC2 — video SDP exceeding the old 4 KB cap now survives the wire.
//   * RC3 — early ICE candidates are buffered, not dropped, so ICE connects.
//   * RC4 — concurrent signaling no longer panics mid-negotiation.
//   * perfect-negotiation — screen-share renegotiation reaches the remote peer.
//
// The observable proof of "remote media arrived" is the `ontrack` side effect
// in crates/web/src/voice.rs: a remote audio track appends
// `<audio id="willow-audio-{peer}">` to the document, and a remote video track
// populates a participant tile `<video>`.

test.describe('voice/video media flow (real WebRTC over loopback)', () => {
  test.describe.configure({ mode: 'serial' });

  // Collect this project's contexts only.
  test.beforeEach(({}, testInfo) => {
    test.skip(
      testInfo.project.name !== 'voice-chrome',
      'voice media tests require the voice-chrome project (fake media devices)',
    );
  });

  /** Create a voice channel from the channel sidebar (kind picker → voice). */
  async function createVoiceChannel(page: Page, name: string) {
    const scope = visibleShell(page);
    await page.locator(`${scope} .channel-add-btn`).first().click();
    await page
      .locator(`${scope} .tree-kind-picker__item`, { hasText: 'voice' })
      .first()
      .click();
    const nameInput = page.locator(`${scope} .tree-slot__input`).first();
    await nameInput.waitFor({ timeout: 5_000 });
    await nameInput.fill(name);
    await nameInput.press('Enter');
    await page
      .locator(`${scope} .voice-channel`, { hasText: name })
      .waitFor({ timeout: 10_000 });
  }

  /** Click a voice channel to join it, then wait for the call page. */
  async function joinVoiceChannel(page: Page, name: string) {
    const scope = visibleShell(page);
    await page
      .locator(`${scope} .voice-channel`, { hasText: name })
      .first()
      .click();
    await page.locator('.call-page').waitFor({ timeout: 15_000 });
  }

  test('both peers receive each other audio track', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await createVoiceChannel(page1, 'voice-room');
      // Wait for the channel to sync to peer 2 over gossip.
      await page2
        .locator(`${visibleShell(page2)} .voice-channel`, { hasText: 'voice-room' })
        .waitFor({ timeout: 20_000 });

      await joinVoiceChannel(page1, 'voice-room');
      await joinVoiceChannel(page2, 'voice-room');

      // The load-bearing assertion: each peer's ontrack fired, meaning the
      // remote audio track crossed the connection. Before the fixes, ICE never
      // connected (dropped candidates) / the offer was dropped (4 KB cap), so
      // this element never appeared.
      await expect(page1.locator('audio[id^="willow-audio-"]'))
        .toHaveCount(1, { timeout: 25_000 });
      await expect(page2.locator('audio[id^="willow-audio-"]'))
        .toHaveCount(1, { timeout: 25_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('remote peer sees a screen share (renegotiation)', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await createVoiceChannel(page1, 'voice-room');
      await page2
        .locator(`${visibleShell(page2)} .voice-channel`, { hasText: 'voice-room' })
        .waitFor({ timeout: 20_000 });

      await joinVoiceChannel(page1, 'voice-room');
      await joinVoiceChannel(page2, 'voice-room');

      // Audio must be flowing before we add video (proves base connection up).
      await expect(page2.locator('audio[id^="willow-audio-"]'))
        .toHaveCount(1, { timeout: 25_000 });

      // Peer 1 shares its (fake) screen. This adds a video track to an already
      // connected peer connection → onnegotiationneeded → renegotiation.
      await page1.locator('.call-btn[title="Share Screen"]').click();

      // Peer 2 is NOT sharing video, so any participant-tile <video> on its
      // page is the remote screen share arriving via ontrack.
      await expect(page2.locator('.participant-tile video').first())
        .toBeVisible({ timeout: 25_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});

import { test, expect } from '@playwright/test';
import { freshStart, createServer } from './helpers';

// Phase 3b T11 — drag-and-drop drop integration.
//
// Spec: docs/specs/2026-04-19-ui-design/files-inline.md §Drag-and-drop.
// Browser-tier (`crates/web/tests/browser.rs` `mod
// phase_3b_drag_overlay`) covers the cheap half — `<DragOverlay>`
// visibility tracks `queue.drag_active`. Real DataTransfer drop
// semantics (carrying actual files) need a real browser; that's
// what this spec pins.
test.describe('Files inline — drag-and-drop drop opens upload dialog', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(
      !testInfo.project.name.startsWith('chromium') &&
        !testInfo.project.name.startsWith('webkit') &&
        !testInfo.project.name.startsWith('firefox'),
      'desktop browsers only — page-level drop overlay does not ship on mobile',
    );
  });

  test('dropping a file opens the upload dialog with that file enqueued', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Drop Test');

    // Construct a real File + DataTransfer in the page context, then
    // fire dragenter (to surface the overlay) and drop (to enqueue +
    // open the dialog) on document.
    await page.evaluate(() => {
      const file = new File(['hello drop world'], 'dropped.txt', {
        type: 'text/plain',
      });
      const dt = new DataTransfer();
      dt.items.add(file);

      // dragenter brings the overlay in.
      const enter = new DragEvent('dragenter', {
        bubbles: true,
        cancelable: true,
        dataTransfer: dt,
      });
      document.dispatchEvent(enter);
    });

    // Overlay should be visible during the drag.
    await expect(page.locator('.drag-overlay')).toBeVisible({ timeout: 5_000 });
    await expect(page.locator('.drag-overlay__label')).toHaveText('drop to attach');

    // Now fire the drop with the same DataTransfer payload.
    await page.evaluate(() => {
      const file = new File(['hello drop world'], 'dropped.txt', {
        type: 'text/plain',
      });
      const dt = new DataTransfer();
      dt.items.add(file);
      const drop = new DragEvent('drop', {
        bubbles: true,
        cancelable: true,
        dataTransfer: dt,
      });
      document.dispatchEvent(drop);
    });

    // Overlay tears down, dialog mounts with the dropped file in the
    // list.
    await expect(page.locator('.drag-overlay')).toHaveCount(0, { timeout: 5_000 });
    await expect(page.locator('.upload-dialog')).toBeVisible({ timeout: 5_000 });
    await expect(
      page.locator('.upload-dialog__filename', { hasText: 'dropped.txt' }),
    ).toBeVisible();
  });
});

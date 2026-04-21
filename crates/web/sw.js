// willow service worker — phase 1f
//
// Spec: docs/specs/2026-04-19-ui-design/notifications.md §OS push payload
//
// install / activate / fetch: unchanged cache-first fallback (future
// Workbox integration pending). The push + notificationclick paths
// enforce the privacy contract: payloads carry only { wake, ref, cat }
// — decryption + composition happen on-device after the push wakes
// the client.

self.addEventListener('install', function(e) { self.skipWaiting(); });
self.addEventListener('activate', function(e) { e.waitUntil(self.clients.claim()); });
self.addEventListener('fetch', function(e) {
    e.respondWith(fetch(e.request).catch(function() { return caches.match(e.request); }));
});

// Push handler — receives opaque { wake, ref, cat }. If a focused
// client is present, postMessage the payload so the in-app Notifier
// can render a toast instead; otherwise fall back to an opaque OS
// notification. Per spec, providers never see plaintext.
self.addEventListener('push', function(event) {
    event.waitUntil((async () => {
        let payload = {};
        if (event.data) {
            try { payload = event.data.json(); }
            catch (_) { payload = { wake: 1 }; }
        }

        const cat = payload.cat || 'msg';
        const ref = payload.ref || null;

        // Focused client wins — no OS notification, just hand the
        // payload to the in-app Notifier via postMessage.
        const clients = await self.clients.matchAll({
            type: 'window',
            includeUncontrolled: true,
        });
        const focused = clients.find((c) => c.focused);
        if (focused) {
            focused.postMessage({
                kind: 'willow-push',
                cat: cat,
                ref: ref,
            });
            return;
        }

        // No focused client — post the opaque default notification.
        // Content-preview composition happens only after local
        // decryption (see notifications.md §Local composition).
        const title = opaqueTitle(cat);
        await self.registration.showNotification(title, {
            body: '',
            tag: cat + ':' + (ref || 'generic'),
            data: { cat: cat, ref: ref },
            icon: '/icon-192.svg',
            badge: '/icon-192.svg',
            silent: false,
            renotify: false,
        });
    })());
});

self.addEventListener('notificationclick', function(event) {
    event.notification.close();
    event.waitUntil((async () => {
        const clients = await self.clients.matchAll({
            type: 'window',
            includeUncontrolled: true,
        });
        const existing = clients.find((c) => 'focus' in c);
        if (existing) {
            // Pass the payload ref so the app can route to the right
            // surface after focus.
            existing.postMessage({
                kind: 'willow-notification-click',
                cat: event.notification.data && event.notification.data.cat,
                ref: event.notification.data && event.notification.data.ref,
            });
            return existing.focus();
        }
        return self.clients.openWindow('/');
    })());
});

// Opaque copy per spec table. Composed notifications are built on the
// client after decryption (task 9 Notifier).
function opaqueTitle(cat) {
    switch (cat) {
        case 'mention':
        case 'msg':
        case 'letter':
            return 'willow — 1 new message';
        default:
            return 'willow';
    }
}

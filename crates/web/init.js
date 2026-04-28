(function() {
    var theme = localStorage.getItem('willow-theme') || 'dark';
    document.documentElement.setAttribute('data-theme', theme);
    // Auto-detect local relay for development.
    var h = location.hostname;
    if (!window.__WILLOW_RELAY_URL && (h === '127.0.0.1' || h === 'localhost')) {
        window.__WILLOW_RELAY_URL = 'http://' + h + ':3340';
    }
    // STUN server overrides for WebRTC voice calls. Privacy-first default:
    // no STUN servers are configured, so voice ICE relies on host candidates
    // plus the iroh relay path — no third-party server learns the user's IP.
    // See issue #179.
    //
    // To opt back into Google's public STUN server (leaks your IP to Google):
    //     window.__WILLOW_STUN_URLS = ['stun:stun.l.google.com:19302'];
    // Or point at a self-hosted STUN server:
    //     window.__WILLOW_STUN_URLS = ['stun:stun.example.com:3478'];
})();

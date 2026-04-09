(function() {
    var theme = localStorage.getItem('willow-theme') || 'dark';
    document.documentElement.setAttribute('data-theme', theme);
    // Auto-detect local relay for development.
    var h = location.hostname;
    if (!window.__WILLOW_RELAY_URL && (h === '127.0.0.1' || h === 'localhost')) {
        window.__WILLOW_RELAY_URL = 'http://' + h + ':3340';
    }
})();

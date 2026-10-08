const seen = {};
const save = () => chrome.storage.session.set({ seen });
chrome.webNavigation.onCommitted.addListener((d) => { if (d.frameId === 0) { seen[d.tabId] = d.url; save(); } });
for (const ev of ['onCreated', 'onUpdated', 'onActivated', 'onRemoved']) {
  chrome.tabs[ev].addListener((...a) => { seen['ev:' + ev] = (seen['ev:' + ev] || 0) + 1; save(); });
}
chrome.runtime.onMessage.addListener((m, s, send) => {
  if (m.ping) send({ pong: true, senderTab: s.tab && s.tab.id, senderUrl: s.url });
  else if (m.seen) send(seen);
  return false;
});

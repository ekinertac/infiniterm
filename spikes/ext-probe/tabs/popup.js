const out = [];
const log = (k, v) => { out.push(k + ': ' + JSON.stringify(v)); document.getElementById('o').textContent = out.join('\n'); };
(async () => {
  const self = await chrome.tabs.getCurrent();
  log('self tab id', self && self.id);
  const seen = await chrome.runtime.sendMessage({ seen: 1 });
  log('bg saw', seen);
  const ids = Object.keys(seen).filter((k) => !k.startsWith('ev:') && +k !== (self && self.id)).map(Number);
  for (const id of ids) {
    try { const t = await chrome.tabs.get(id); log('tabs.get ' + id, { url: t.url, title: t.title, active: t.active, windowId: t.windowId }); } catch (e) { log('tabs.get ' + id + ' err', String(e)); }
    try { const r = await chrome.scripting.executeScript({ target: { tabId: id }, func: () => document.title + ' | bg=' + getComputedStyle(document.body).backgroundColor }); log('executeScript ' + id, r.map((x) => x.result)); } catch (e) { log('executeScript ' + id + ' err', String(e)); }
    try { const r = await chrome.tabs.sendMessage(id, { hi: 1 }); log('tabs.sendMessage ' + id, r); } catch (e) { log('tabs.sendMessage ' + id + ' err', String(e)); }
  }
  try { log('tabs.query all', (await chrome.tabs.query({})).length); } catch (e) { log('query err', String(e)); }
})();

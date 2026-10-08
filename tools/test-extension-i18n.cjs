// Run with node --test tools/test-extension-i18n.cjs. No browser or network needed.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '..');
const read = (p) => fs.readFileSync(path.join(root, p), 'utf8');
const locales = Object.fromEntries(['en', 'zh_CN'].map(l => [l, JSON.parse(read(`extension/_locales/${l}/messages.json`))]));
const keys = (o) => Object.keys(o || {}).sort();

test('Chrome catalogs have identical keys, placeholders and valid substitutions', () => {
  assert.deepEqual(keys(locales.en), keys(locales.zh_CN));
  for (const key of keys(locales.en)) {
    assert.deepEqual(keys(locales.en[key].placeholders), keys(locales.zh_CN[key].placeholders));
    for (const catalog of Object.values(locales)) {
      const entry = catalog[key];
      assert.ok(entry.message.trim(), key);
      const references = [...entry.message.matchAll(/\$([A-Za-z_]+)\$/g)].map(m => m[1].toLowerCase()).sort();
      assert.deepEqual(references, keys(entry.placeholders), key);
      for (const [name, placeholder] of Object.entries(entry.placeholders || {})) {
        assert.equal(placeholder.content, locales.en[key].placeholders[name].content);
      }
    }
  }
});

test('manifest and popup keys exist; catalogs are embedded in the desktop binary', () => {
  const manifest = JSON.parse(read('extension/manifest.json'));
  assert.equal(manifest.default_locale, 'zh_CN');
  assert.equal(manifest.name, 'GaGaDown');
  assert.equal(manifest.action.default_title, 'GaGaDown');
  for (const [, key] of JSON.stringify(manifest).matchAll(/__MSG_(\w+)__/g)) assert.ok(locales.en[key], key);
  for (const [, key] of read('extension/popup.html').matchAll(/data-i18n="(\w+)"/g)) assert.ok(locales.en[key], key);
  const app = read('crates/gagadown-app/src/main.rs');
  for (const locale of keys(locales)) assert.ok(app.includes(`include_bytes!("../../../extension/_locales/${locale}/messages.json")`));
  for (const file of ['popup.html', 'popup.js', 'background.js', 'manifest.json']) {
    assert.doesNotMatch(read(`extension/${file}`), /[\u3400-\u9fff]/u, file);
  }
});

for (const locale of keys(locales)) {
  for (const status of ['connected', 'offline', 'rejected']) {
    test(`popup ${locale}: ${status}, interpolation and toggle`, async () => {
      const elements = new Map();
      const element = (id) => {
        if (!elements.has(id)) {
          const classes = new Set();
          elements.set(id, { textContent: '', dataset: {}, classList: {
            contains: c => classes.has(c),
            toggle: (c, on) => on ? classes.add(c) : classes.delete(c),
          }, addEventListener: (_event, fn) => { elements.get(id).click = fn; } });
        }
        return elements.get(id);
      };
      const staticElements = ['checking', 'takeover', 'minimumSize'].map(key => {
        const e = element(key === 'checking' ? 'stateText' : key);
        e.dataset.i18n = key;
        return e;
      });
      const stored = [];
      const document = { documentElement: {}, getElementById: element, querySelectorAll: () => staticElements };
      const chrome = {
        i18n: { getUILanguage: () => locale.replace('_', '-'), getMessage: (key, value) => {
          assert.ok(locales[locale][key], key);
          return locales[locale][key].message.replace('$VERSION$', value);
        } },
        storage: { local: { get: async () => ({ enabled: true, minSize: 1048576 }), set: async v => stored.push(v.enabled) } },
        runtime: { getManifest: () => ({ version: '1.2.3' }), sendMessage: async () => {
          if (status === 'rejected') throw new Error('offline');
          return status === 'connected' ? { takeover_min_size: 2097152 } : null;
        } },
      };
      vm.runInNewContext(read('extension/popup.js'), { document, chrome });
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(document.documentElement.lang, locale.replace('_', '-'));
      assert.equal(element('stateText').textContent, locales[locale][status === 'connected' ? 'connected' : 'notRunning'].message);
      assert.ok(element('ver').textContent.includes('1.2.3'));
      assert.equal(element('minSize').textContent, status === 'connected' ? '2 MB' : '1 MB');
      assert.equal(element('takeover').textContent, locales[locale].takeover.message);
      await element('toggleRow').click();
      assert.deepEqual(stored, [false]);
    });
  }
}

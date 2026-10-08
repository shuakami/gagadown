// Tests a CI-built CLI against a bounded loopback fixture; never contacts a real source.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const binary = process.env.GAGADOWN_CLI;
assert.ok(binary, 'Set GAGADOWN_CLI to the CI-built executable');

function run(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { env: { ...process.env, NO_PROXY: 'localhost,127.0.0.1' } });
    let output = '';
    const timer = setTimeout(() => { child.kill(); reject(new Error('CLI exceeded 30-second budget')); }, 30000);
    child.stdout.on('data', b => { output += b; });
    child.stderr.on('data', b => { output += b; });
    child.on('error', e => { clearTimeout(timer); reject(e); });
    child.on('close', code => { clearTimeout(timer); resolve({ code, output }); });
  });
}

for (const language of ['en', 'zh-CN']) {
  test(`CLI ${language}: help, small download, explicit state preservation`, async () => {
    const help = await run(['--language', language, 'get', '--help']);
    assert.equal(help.code, 0, help.output);
    assert.ok(help.output.includes(language === 'en' ? 'Download folder' : '下载目录'));
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'gagadown-cli-test-'));
    const data = path.join(root, 'data');
    await fs.mkdir(data);
    const sentinel = path.join(data, 'preserve.txt');
    await fs.writeFile(sentinel, 'caller-owned state');
    const payload = Buffer.alloc(64 * 1024, 0x61);
    const server = http.createServer((req, res) => {
      let start = 0, end = payload.length - 1;
      const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || '');
      if (range) {
        start = Number(range[1]);
        end = range[2] ? Math.min(Number(range[2]), end) : end;
        res.statusCode = 206;
        res.setHeader('Content-Range', `bytes ${start}-${end}/${payload.length}`);
      }
      res.setHeader('Accept-Ranges', 'bytes');
      res.setHeader('Content-Type', 'application/octet-stream');
      res.setHeader('Content-Length', end - start + 1);
      res.end(req.method === 'HEAD' ? undefined : payload.subarray(start, end + 1));
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    try {
      const result = await run(['--language', language, 'get', `http://127.0.0.1:${server.address().port}/fixture.bin`,
        '--data-dir', data, '--dir', path.join(root, 'files'), '--initial', '1', '--max-connections', '1', '--direct-only',
        '--sha256', crypto.createHash('sha256').update(payload).digest('hex')]);
      assert.equal(result.code, 0, result.output);
      assert.ok(result.output.includes(language === 'en' ? 'Downloaded' : '已下载'), result.output);
      assert.equal(await fs.readFile(sentinel, 'utf8'), 'caller-owned state');
      assert.deepEqual(await fs.readFile(path.join(root, 'files', 'fixture.bin')), payload);
    } finally {
      await new Promise(resolve => server.close(resolve));
      await fs.rm(root, { recursive: true, force: true });
    }
  });
}

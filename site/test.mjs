import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const expectedVercelIgnore = `# Deploy only the static workshop site's build inputs.
/*
!site
!skills
!vercel.json

# Include only source consumed by site/build.mjs.
site/*
!site/build.mjs
!site/index.html
!site/site.js
!site/styles.css

# The site build reads only the canonical setup skill.
skills/*
!skills/mosaico-setup
skills/mosaico-setup/*
!skills/mosaico-setup/SKILL.md
`;

const [html, css, script, setup, builtSetup, vercelIgnore, gitignore] = await Promise.all([
  readFile(new URL('./index.html', import.meta.url), 'utf8'),
  readFile(new URL('./styles.css', import.meta.url), 'utf8'),
  readFile(new URL('./site.js', import.meta.url), 'utf8'),
  readFile(new URL('../skills/mosaico-setup/SKILL.md', import.meta.url), 'utf8'),
  readFile(new URL('./dist/SETUP.md', import.meta.url), 'utf8'),
  readFile(new URL('../.vercelignore', import.meta.url), 'utf8'),
  readFile(new URL('../.gitignore', import.meta.url), 'utf8'),
]);

assert.equal(builtSetup, setup, 'built SETUP.md must match the canonical skill');
assert.equal(
  vercelIgnore,
  expectedVercelIgnore,
  'Vercel uploads must stay limited to the static site and canonical setup skill',
);
assert.match(gitignore, /^\.vercel\/$/m, 'Vercel project links must remain untracked');
assert.match(
  html,
  /Tell your agent: "Go to https:\/\/mosaico\.f7z\.io\/SETUP\.md and follow the instructions\."/,
);
assert.match(css, /@media \(max-width: 800px\)/, 'mobile layout is explicit');
assert.match(css, /prefers-color-scheme: dark/, 'dark theme is explicit');
assert.match(css, /prefers-reduced-motion: reduce/, 'reduced motion is explicit');
assert.doesNotMatch(html, /[—–]/, 'visible copy must not contain long dashes');

async function exerciseCopy(clipboard) {
  let click;
  const button = {
    disabled: false,
    textContent: 'Copy setup prompt',
    addEventListener(name, handler) {
      assert.equal(name, 'click');
      click = handler;
    },
  };
  const status = { textContent: '' };
  const document = {
    querySelector(selector) {
      return selector === '[data-copy-setup]' ? button : status;
    },
  };

  vm.runInNewContext(script, { document, navigator: { clipboard } });
  await click();
  return { button, status };
}

const copied = await exerciseCopy({ writeText: async () => {} });
assert.equal(copied.button.textContent, 'Copied');
assert.equal(copied.status.textContent, 'Paste the prompt into your coding agent.');
assert.equal(copied.button.disabled, false);

const fallback = await exerciseCopy(undefined);
assert.equal(fallback.button.textContent, 'Copy setup prompt');
assert.match(fallback.status.textContent, /^Copy this: Go to https:\/\//);
assert.equal(fallback.button.disabled, false);

console.log('site tests passed');

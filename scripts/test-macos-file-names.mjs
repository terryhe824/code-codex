import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {build} from '../packages/explorer-ui/node_modules/esbuild/lib/main.js';

const root = fileURLToPath(new URL('../packages/explorer-ui/src/', import.meta.url));
const result = await build({
  stdin: {
    contents: `
      export {contextActionError,entryNameValidationError} from "./features/file-operations/context-menu";
      export {captureExternalDropCandidates,ensureDistinctExternalRootNames,externalImportError,normalizeExternalDropReadError,resolveExternalDropRoots} from "./features/file-operations/external-import";
      export {editSaveError} from "./features/preview/responses";
      export {PreviewWorkspaceController} from "./features/workspace/preview-workspace";
      export {joinAbsolutePath} from "./core/bridge/response-validation";
      export {TreeModel} from "./tree-model";
      import {pluginModules} from "./plugin-runtime";
      import {normalizeGitDiff} from "./git-history-runtime";
      pluginModules().set("git-history", {api: 1, exports: {normalizeGitDiff}});
    `,
    resolveDir: root,
  },
  bundle: true,
  banner: {js: 'const window = {};'},
  define: {__CODE_CODEX_VERSION__: '"test"'},
  format: 'esm',
  write: false,
});
const {captureExternalDropCandidates, contextActionError, editSaveError, entryNameValidationError, ensureDistinctExternalRootNames, externalImportError, normalizeExternalDropReadError, PreviewWorkspaceController, resolveExternalDropRoots, TreeModel, joinAbsolutePath} = await import(
  'data:text/javascript;base64,' + Buffer.from(result.outputFiles[0].text + '\n//# sourceURL=macos-file-names-test-bundle.js').toString('base64')
);

test('macOS preserves case-distinct import roots and rejects identical roots', () => {
  return withPlatform('MacIntel', () => {
    assert.doesNotThrow(() => ensureDistinctExternalRootNames([{name: 'Name.txt'}, {name: 'name.txt'}]));
    assert.throws(() => ensureDistinctExternalRootNames([{name: 'Name.txt'}, {name: 'Name.txt'}]), {code: 'CONFLICT'});
  });
});

test('Windows retains case-insensitive import duplicate checking', () => {
  return withPlatform('Win32', () => {
    assert.throws(() => ensureDistinctExternalRootNames([{name: 'Name.txt'}, {name: 'name.txt'}]), {code: 'CONFLICT'});
  });
});

test('platform file actions preserve access-denied errors and saved drafts', async () => {
  for (const [platform, os] of [['MacIntel', 'macOS'], ['Win32', 'Windows']]) {
    await withPlatform(platform, () => {
      const permission = normalizeExternalDropReadError(new DOMException('private path', 'NotAllowedError'));
      assert.equal(permission.code, 'ACCESS_DENIED');
      assert.equal(permission.message, `${os} could not read one of the dropped items.`);
      assert.equal(externalImportError(permission, 0), `${os} denied access to one of the dropped items.`);
      assert.match(contextActionError('reveal', permission), new RegExp(os));
      assert.equal(editSaveError(permission), `${os} denied permission to save this file. Your draft has been kept.`);
    });
  }
});

test('macOS drop uses Files granted by Finder without opening the modern handle', async () => {
  await withPlatform('MacIntel', async () => {
    const file = new File(['test bytes'], 'Finder 导入验收.txt');
    let handleReads = 0;
    const item = {
      kind: 'file',
      getAsFile: () => null,
      getAsFileSystemHandle: async () => ({
        kind: 'file',
        name: file.name,
        getFile() {
          handleReads += 1;
          throw new DOMException('FileSystemFileHandle.getFile is unavailable', 'SecurityError');
        },
      }),
    };
    // A transfer can include string items. Its FileList contains only file entries.
    const candidates = captureExternalDropCandidates({items: [{kind: 'string'}, item], files: [file]});
    const [root] = await resolveExternalDropRoots(candidates, () => {});
    assert.equal(root.name, file.name);
    assert.equal(await root.file.text(), 'test bytes');
    assert.equal(handleReads, 0);
    await assert.rejects(
      resolveExternalDropRoots(captureExternalDropCandidates({items: [item], files: []}), () => {}),
      {code: 'ACCESS_DENIED'}
    );
  });
});

test('macOS drop still traverses directory handles when a File is also exposed', async () => {
  await withPlatform('MacIntel', async () => {
    const placeholder = new File([], 'folder');
    const nested = new File(['test bytes'], 'nested.txt');
    const item = {
      kind: 'file',
      getAsFile: () => placeholder,
      getAsFileSystemHandle: async () => ({
        kind: 'directory',
        name: 'folder',
        async *entries() {
          yield [nested.name, {kind: 'file', name: nested.name, getFile: async () => nested}];
        },
      }),
    };
    const [root] = await resolveExternalDropRoots(
      captureExternalDropCandidates({items: [item], files: [placeholder]}), () => {},
    );
    assert.equal(root.kind, 'directory');
    assert.equal(root.members[0].relativePath, nested.name);
    assert.equal(await root.members[0].file.text(), 'test bytes');
  });
});

test('macOS rejects a known directory when traversal is unavailable', async () => {
  await withPlatform('MacIntel', async () => {
    const placeholder = new File([], 'unreadable-folder');
    const item = {
      kind: 'file',
      getAsFile: () => null,
      getAsFileSystemHandle: async () => ({kind: 'directory', name: placeholder.name}),
    };
    const candidates = captureExternalDropCandidates({items: [item], files: [placeholder]});
    await assert.rejects(resolveExternalDropRoots(candidates, () => {}), {code: 'ACCESS_DENIED'});

    const emptyFile = new File([], 'empty.txt');
    const regular = {...item, getAsFileSystemHandle: async () => ({kind: 'file', name: emptyFile.name})};
    const [root] = await resolveExternalDropRoots(
      captureExternalDropCandidates({items: [regular], files: [emptyFile]}), () => {},
    );
    assert.equal(root.kind, 'file');
    assert.equal(root.file, emptyFile);
    assert.equal(root.sizeBytes, 0);
  });
});

async function withPlatform(platform, callback) {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  Object.defineProperty(globalThis, 'navigator', {configurable: true, value: {platform}});
  try {
    await callback();
  } finally {
    if (original) Object.defineProperty(globalThis, 'navigator', original);
    else delete globalThis.navigator;
  }
}

test('macOS accepts native filename characters and rejects actual separators', () => {
  return withPlatform('MacIntel', () => {
    for (const name of ['NUL.txt', '中文\\草稿:é*.txt. ', ' ']) {
      assert.equal(entryNameValidationError(name), undefined, name);
    }
    for (const name of ['', '.', '..', 'nested/file', 'nul\u0000char', 'e\u0301'.repeat(128)]) {
      assert.ok(entryNameValidationError(name), name);
    }
  });
});

test('the macOS workspace tree keeps backslash and colon in a literal filename', () => {
  return withPlatform('MacIntel', () => {
    const model = new TreeModel();
    const name = '中文\\草稿:é*.txt. ';
    const file = {id: name, relativePath: name, name, kind: 'file'};
    model.commitLoad('', {entries: [file]});
    assert.deepEqual(model.getNode(name), file);
  });
});

test('existing Windows filename validation keeps Windows-only restrictions', () => {
  return withPlatform('Win32', () => {
    for (const name of ['NUL.txt', 'nested\\file', 'ads:stream', 'end. ']) {
      assert.ok(entryNameValidationError(name), name);
    }
    assert.equal(entryNameValidationError('文件.txt'), undefined);
  });
});

test('macOS Git preview keeps a literal backslash in its tab and request', () => {
  return withPlatform('MacIntel', async () => {
    const path = 'src/文档\\draft.txt';
    const hash = '0123456789abcdef';
    const requested = [];
    const host = {
      state: 'ready',
      context: {},
      bridge: {
        available: true,
        async request(method, params) {
          requested.push({method, params});
          return {path: '', content: 'diff', truncated: false};
        },
      },
      mainPreviewSurface: {isConnected: true},
      ensureMainPreview: () => ({}),
      editingPath: null,
      element: {dataset: {}},
      renderVisible() {},
      announce() {},
    };
    const controller = new PreviewWorkspaceController(host);
    controller.syncMainPreview = () => {};
    await controller.openGitDiff(hash, path);
    assert.deepEqual(requested, [{method: 'explorer.git.diff', params: {hash, path}}]);
    assert.equal(controller.previewTabs[0].name, '文档\\draft.txt');
    assert.equal(controller.previewTabs[0].path, `git-diff://${hash}/${path}`);
    assert.equal(controller.previewTabs[0].view.sourcePath, path);
  });
});


test('absolute copy path preserves literal Mac backslashes and Windows native separators', async () => {
  await withPlatform('MacIntel', () => {
    assert.equal(joinAbsolutePath('/Users/test/one\\two/', 'drafts\\notes.txt'), '/Users/test/one\\two/drafts\\notes.txt');
    assert.equal(joinAbsolutePath('/Users/test/work/', 'nested/file.txt'), '/Users/test/work/nested/file.txt');
  });
  await withPlatform('Win32', () => {
    assert.equal(joinAbsolutePath('C:\\work\\', 'nested/file.txt'), 'C:\\work\\nested\\file.txt');
    assert.equal(joinAbsolutePath('\\\\server\\share\\', 'nested/file.txt'), '\\\\server\\share\\nested\\file.txt');
  });
});

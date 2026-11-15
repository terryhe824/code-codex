import assert from 'node:assert/strict';
import {mkdirSync, writeFileSync} from 'node:fs';
import {createServer} from 'node:http';
import {resolve} from 'node:path';
import {build} from '../packages/explorer-ui/node_modules/esbuild/lib/main.js';
import {launchFixture} from './browser-fixture.mjs';

// Real production components, isolated browser and synthetic IME events.
// This does not claim that a macOS input-method candidate window was exercised.
const root = resolve(import.meta.dirname, '..');
const bundle = await build({bundle: true, write: false, format: 'iife',
  define: {__CODE_CODEX_VERSION__: '"ime-test"', __CODE_CODEX_PLUGIN_CATALOG__: '[]'},
  stdin: {resolveDir: resolve(root, 'packages/explorer-ui/src'), loader: 'ts', contents: `
    import {registerMainPreviewElement,MAIN_PREVIEW_TAG} from './main-preview';
    import {TreeInteractionController} from './features/workspace/tree-interaction';
    registerMainPreviewElement(); window.api={MAIN_PREVIEW_TAG,TreeInteractionController};
  `}});
const html = '<!doctype html><meta charset="utf-8"><input id="filter"><div id="tree" tabindex="0"></div><script src="/fixture.js"></script>';
const server = createServer((req, res) => {
  res.setHeader('Content-Type', req.url === '/fixture.js' ? 'text/javascript' : 'text/html');
  res.end(req.url === '/fixture.js' ? bundle.outputFiles[0].text : html);
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
let browser;
const results = [];
try {
  browser = await launchFixture(`http://127.0.0.1:${server.address().port}/`);
  await browser.evaluate(`(()=>{
    window.errors=[];addEventListener('error',e=>errors.push(e.message));
    window.input=document.querySelector('#filter');window.tree=document.querySelector('#tree');
    window.controller=new api.TreeInteractionController({fileFilterInput:input,treeShell:tree,announce:()=>{}});
    // Rendering is unrelated to keyboard ownership; keep the fixture boundary explicit.
    controller.renderTree=()=>{}; controller.focusIndex=()=>tree.focus();controller.rows=[{}];
    input.addEventListener('keydown',e=>controller.onFileFilterKeyDown(e));
    window.viewer=document.createElement(api.MAIN_PREVIEW_TAG);document.body.append(viewer);
    window.activations=[];window.saves=[];
    viewer.addEventListener('cle-main-preview-activate',e=>activations.push(e.detail));
    viewer.addEventListener('cle-main-preview-save',e=>saves.push(e.detail));
    const view={kind:'text',path:'ime.txt',name:'ime.txt',text:'中文草稿',sizeBytes:12,truncated:false,editable:true,version:'a'.repeat(64),lineEnding:'lf'};
    viewer.setState({activePath:view.path,tabs:[view],enabledPreviewers:[],editor:{path:view.path,draft:view.text,dirty:true,saving:false}});
    window.editor=viewer.shadowRoot.querySelector('textarea');
    window.fire=(target,key,options={})=>{const e=new KeyboardEvent('keydown',{key,bubbles:true,composed:true,cancelable:true,...options});target.dispatchEvent(e);return e.defaultPrevented;};
  })()`);
  const check = async (name, expression) => {
    const passed = await browser.evaluate(expression);
    results.push({name, passed});
  };
  for (const mode of [{isComposing:true}, {keyCode:229}]) {
    for (const key of ['Escape','ArrowDown']) {
      await check(`Filter keeps IME ${key} ${JSON.stringify(mode)}`, `(()=>{controller.rows=[{}];input.value='zhong';controller.fileFilterQuery='zhong';input.focus();const prevented=fire(input,${JSON.stringify(key)},${JSON.stringify(mode)});return !prevented&&document.activeElement===input&&input.value==='zhong'&&controller.fileFilterQuery==='zhong';})()`);
    }
    for (const options of [{key:'Escape'}, {key:'s',metaKey:true}, {key:'s',ctrlKey:true}]) {
      await check(`Editor keeps IME key ${JSON.stringify({...mode,...options})}`, `(()=>{const a=activations.length,s=saves.length;editor.focus();const prevented=fire(editor,${JSON.stringify(options.key)},${JSON.stringify({...mode,...options})});return !prevented&&activations.length===a&&saves.length===s&&editor.value==='中文草稿';})()`);
    }
  }
  await check('Committed filter Escape clears and ArrowDown navigates', `(()=>{input.value='中文';controller.fileFilterQuery='中文';input.focus();const cleared=fire(input,'Escape')&&input.value===''&&controller.fileFilterQuery==='';controller.rows=[{}];return cleared&&fire(input,'ArrowDown')&&document.activeElement===tree;})()`);
  await check('Committed Cmd-S and Ctrl-S each save once; Escape activates conversation', `(()=>{saves.length=0;activations.length=0;return fire(editor,'s',{metaKey:true})&&fire(editor,'s',{ctrlKey:true})&&saves.length===2&&saves.every(x=>x.path==='ime.txt')&&fire(editor,'Escape')&&activations.length===1&&activations[0].kind==='conversation'&&editor.value==='中文草稿';})()`);
  await check('No browser event errors', 'errors.length===0');
  const out=resolve(root,'artifacts/workspace-ime');mkdirSync(out,{recursive:true});
  writeFileSync(resolve(out,'result.json'),JSON.stringify({scope:'Production components; synthetic composition events; no real OS IME or file writes',results},null,2));
  console.log(JSON.stringify(results,null,2));
  assert.equal(results.filter(x=>!x.passed).length,0,'IME keys must stay with the input method');
} finally {
  if(browser)await browser.close();
  await new Promise(r=>server.close(r));
}

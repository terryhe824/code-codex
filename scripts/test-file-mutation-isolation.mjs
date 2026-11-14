import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import ts from '../packages/explorer-ui/node_modules/typescript/lib/typescript.js';

// Execute the real production methods with bounded fake IO; do not copy their
// implementation into a test. DOM menus and renderer wiring have browser tests.
const source=await readFile(new URL('../packages/explorer-ui/src/features/workspace/file-actions.ts',import.meta.url),'utf8');
const sf=ts.createSourceFile('file-actions.ts',source,ts.ScriptTarget.Latest,true),cl=sf.statements.find(ts.isClassDeclaration);
const wanted=new Set(['requestEntryAction','pasteFiles','deleteSelectedFiles','refreshDirectoryIfLoaded']);
const methods=cl.members.filter(m=>ts.isMethodDeclaration(m)&&wanted.has(m.name.getText(sf))).map(m=>m.getText(sf));
assert.equal(methods.length,wanted.size);
const js=ts.transpileModule(`const recordCaughtRuntimeError=()=>{};const contextActionError=(action,error)=>error.message;class BridgeUnavailableError extends Error{};export class Subject{constructor(private readonly host:any){} ${methods.join('\n')}}`,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText;
const {Subject}=await import('data:text/javascript;base64,'+Buffer.from(js).toString('base64'));
function fixture(){let active;const calls=[],clipboard={paths:['one'],operation:'cut'},context={threadId:'a'},session={context,threadId:'a',generation:1};const bridge={available:true,async request(method,params){calls.push({method,params});}};session.bridge=bridge;const host={workspaceSession:session,fileClipboard:clipboard,rows:[],focusedIndex:0,selectedPaths:new Set(['one','two']),mutations:{acquire(){if(active)return;const token=active={};return{release(){if(active===token)active=undefined;}};}},canUseBridge:(b,g)=>session.bridge===b&&session.generation===g,clearFileClipboard:expected=>{if(host.fileClipboard===expected)host.fileClipboard=undefined;},clearSelection:()=>calls.push('clear selection'),model:{hasLoaded:()=>false},loadDirectory:async()=>calls.push('refresh directory'),refreshLoadedDirectories:async()=>calls.push('refresh root'),showActionNotice:()=>calls.push('notice')};return{subject:new Subject(host),host,session,bridge,calls,clipboard,busy:()=>!!active};}
test('a late cut completion cannot clear newer clipboard content',async()=>{const f=fixture();const newer={paths:['new'],operation:'cut'};f.bridge.request=async()=>{f.host.fileClipboard=newer;};await f.subject.pasteFiles();assert.equal(f.host.fileClipboard,newer);assert.equal(f.busy(),false);});
test('workspace change stops remaining delete requests and does not clear or refresh the new selection',async()=>{const f=fixture();f.bridge.request=async(method,params)=>{f.calls.push({method,params});f.session.context={threadId:'b'};f.session.threadId='b';f.session.generation++;};await f.subject.deleteSelectedFiles();assert.equal(f.calls.length,1);assert.equal(f.calls[0].params.relativePath,'one');assert.deepEqual([...f.host.selectedPaths],['one','two']);assert.equal(f.busy(),false);});
test('late paste completion from an old workspace cannot clear clipboard or refresh a new workspace',async()=>{const f=fixture();f.bridge.request=async()=>{f.session.generation++;};await f.subject.pasteFiles();assert.equal(f.host.fileClipboard,f.clipboard);assert.deepEqual(f.calls,[]);assert.equal(f.busy(),false);});
test('shared busy gate prevents keyboard paste and delete from starting another write',async()=>{const f=fixture(),lease=f.host.mutations.acquire();await f.subject.pasteFiles();await f.subject.deleteSelectedFiles();assert.deepEqual(f.calls,[]);assert.equal(f.busy(),true);lease.release();});
test('failed delete releases its lease and reports the original failure without retrying later paths',async()=>{const f=fixture();let calls=0;f.bridge.request=async()=>{calls++;throw Error('disk denied');};await f.subject.deleteSelectedFiles();assert.equal(calls,1);assert.equal(f.busy(),false);assert.deepEqual(f.calls,['notice']);});

const explorerSource = await readFile(new URL('../packages/explorer-ui/src/app/explorer-element.ts', import.meta.url), 'utf8');
const explorerAst = ts.createSourceFile('explorer-element.ts', explorerSource, ts.ScriptTarget.Latest, true);
const explorerClass = explorerAst.statements.find(ts.isClassDeclaration);
const applyChanges = explorerClass.members.find(member => ts.isMethodDeclaration(member) && member.name.getText(explorerAst) === '#applyChanges');
const watcherConsumer = ts.transpileModule(`
const runtimeEvent = () => {};
const asRecord = value => value;
const normalizeChange = value => value;
export class WatcherConsumer {
  #previeweditingSettings = {drafts: {}};
  #previewWorkspace;
  #treeinteractionSettings = {model: {applyChange: () => [], loadedDirectories: () => []}, renderTree() {}};
  #pendingMarks = new Map();
  #scheduleChangeExpiry() {}
  #scheduleRefresh() {}
  #announce() {}
  constructor(openPreview) {
    this.#previewWorkspace = {markPreviewModified: path => openPreview.reload(path)};
  }
  apply(changes) { this.#applyChanges(changes); }
  ${applyChanges.getText(explorerAst)}
}
`, {compilerOptions: {target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext}});
const {WatcherConsumer} = await import('data:text/javascript;base64,' + Buffer.from(watcherConsumer.outputText).toString('base64'));

test('an Added watcher event refreshes an existing file preview', () => {
  const reloaded = [];
  const consumer = new WatcherConsumer({
    reload(path) {
      if (path === 'open.txt') reloaded.push(path);
    },
  });
  consumer.apply([{kind: 'added', relativePath: 'open.txt'}, {kind: 'added', relativePath: 'new.txt'}]);
  assert.deepEqual(reloaded, ['open.txt']);
});

const onNotification = explorerClass.members.find(member => ts.isMethodDeclaration(member) && member.name.getText(explorerAst) === '#onNotification');
const notificationConsumer = ts.transpileModule(`
const runtimeEvent = () => {};
const asRecord = value => value;
const getBootstrapConfig = () => ({});
export class NotificationConsumer {
  #updatesSettings = {};
  #showActionProgress() {}
  #updateMessage = {};
  #workspaceSession = {context: {threadId: 'same'}};
  #state = 'ready';
  #applyChanges() {}
  #switchThread() {}
  #nativeReconnectMarker = {};
  #setState() {}
  #previeweditingSettings = {drafts: {path: 'draft.txt', text: 'unsaved'}};
  #previewWorkspace = {tabStore: {items: [{path:'draft.txt'}, {path:'preview.txt'}]}, markPreviewModified: path => this.invalidated.push(path)};
  #refreshLoadedDirectories() {this.refreshes++; return Promise.resolve();}
  refreshes = 0; reconnects = 0; invalidated = [];
  get draft() {return this.#previeweditingSettings.drafts;}
  get tabs() {return this.#previewWorkspace.tabStore.items;}
  reconnectNative() {this.reconnects++;}
  notify(method, params) {this.#onNotification(method, params);}
  ${onNotification.getText(explorerAst)}
}`, {compilerOptions: {target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext}});
const {NotificationConsumer} = await import('data:text/javascript;base64,' + Buffer.from(notificationConsumer.outputText).toString('base64'));
test('ambiguous filesystem rename rescans the tree without discarding drafts or preview tabs', () => {
  const consumer = new NotificationConsumer();
  const draft = consumer.draft, tabs = consumer.tabs;
  consumer.notify('explorer.resync', {scope: 'filesystem'});
  assert.equal(consumer.reconnects, 0);
  assert.equal(consumer.refreshes, 1);
  assert.equal(consumer.draft, draft);
  assert.equal(consumer.draft.text, 'unsaved');
  assert.equal(consumer.tabs, tabs);
  assert.deepEqual(consumer.invalidated, ['preview.txt']);
  consumer.notify('explorer.resync', {});
  assert.equal(consumer.reconnects, 1);
});

// Only structural observations from the separately owned acceptance page.
let input='';for await(const chunk of process.stdin)input+=chunk;
const {endpoint,mark=false}=JSON.parse(input);const url=new URL(endpoint);
if(url.hostname!=='127.0.0.1'||url.protocol!=='ws:')throw Error('Expected owned loopback endpoint');
const socket=new WebSocket(endpoint);
const expression=`(()=>{
 if(${Boolean(mark)}){
  const banner=document.createElement('div');banner.id='code-codex-test-session';
  banner.textContent='Code-Codex · 独立测试窗口 · 请在此手动登录';
  banner.style.cssText='position:fixed;top:10px;left:50%;transform:translateX(-50%);z-index:2147483647;background:#fff1c2;color:#332400;padding:5px 12px;border-radius:8px;font:12px system-ui;pointer-events:none';
  document.documentElement.appendChild(banner);
 }
 return {readyState:document.readyState,hasBody:!!document.body,
  mainCount:document.querySelectorAll('main').length,
  sidebarCount:document.querySelectorAll('[data-sidebar]').length,
  explorerCount:[...document.querySelectorAll('*')].filter(e=>e.tagName.toLowerCase().startsWith('code-codex-v')).length,
  composerCount:document.querySelectorAll('[contenteditable="true"],textarea').length};
})()`;
const result=await new Promise((resolve,reject)=>{
 const timer=setTimeout(()=>reject(Error('session page timeout')),5000);
 socket.onerror=()=>{clearTimeout(timer);reject(Error('session page unavailable'));};
 socket.onopen=()=>socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression,returnByValue:true}}));
 socket.onmessage=e=>{const value=JSON.parse(e.data);if(value.id===1){clearTimeout(timer);resolve(value.result?.result?.value??{error:true});}};
});socket.close();console.log(JSON.stringify(result));

let input='';for await(const chunk of process.stdin)input+=chunk;
const {endpoint}=JSON.parse(input);const u=new URL(endpoint);
if(u.hostname!=='127.0.0.1'||u.protocol!=='ws:')throw Error('Not owned loopback');
const ws=new WebSocket(endpoint);
const result=await new Promise((resolve,reject)=>{
 const timer=setTimeout(()=>reject(Error('probe timeout')),5000);
 ws.onerror=()=>{clearTimeout(timer);reject(Error('websocket failed'));};
 ws.onopen=()=>ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:"(()=>{const p=document.createElement('span');p.id='code-codex-macos-probe';document.documentElement.appendChild(p);const injected=!!document.getElementById(p.id);p.remove();return {injected,restored:!document.getElementById(p.id),readyState:document.readyState,requireAvailable:typeof require==='function',processAvailable:typeof process!=='undefined',hasBody:!!document.body,explorerMounted:[...document.querySelectorAll('*')].some(e=>e.tagName.toLowerCase().startsWith('code-codex-v')),loginPageObserved:/登录 ChatGPT|Log in to ChatGPT/.test(document.body?.innerText??''),explorerCount:[...document.querySelectorAll('*')].filter(e=>e.tagName.toLowerCase().startsWith('code-codex-v')).length}})()",returnByValue:true}}));
 ws.onmessage=e=>{const v=JSON.parse(e.data);if(v.id===1){clearTimeout(timer);resolve(v.result?.result?.value??{error:true});}};
});ws.close();console.log(JSON.stringify(result));

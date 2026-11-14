import argparse, json, os, pathlib, select, signal, socket, subprocess, tempfile, time, urllib.request
from contextlib import ExitStack


from owned_process import OwnedGroup


parser=argparse.ArgumentParser(description='Launch an isolated signed official Mac host and inspect only its own test page.')
parser.add_argument('--core',type=pathlib.Path)
parser.add_argument('--app',type=pathlib.Path,default=pathlib.Path('/Applications/ChatGPT.app'))
parser.add_argument('--report',type=pathlib.Path,required=True)
args=parser.parse_args()
args.report.parent.mkdir(parents=True,exist_ok=True)
if args.core: args.core=args.core.resolve(strict=True)
repo=pathlib.Path(__file__).resolve().parents[2]
subprocess.run(['/usr/bin/codesign','--verify','--strict','--deep','-R','=identifier "com.openai.codex" and anchor apple generic and certificate leaf[subject.OU] = "2DC432GLL2"',str(args.app)],check=True,capture_output=True)
import plistlib
info=plistlib.loads((args.app/'Contents/Info.plist').read_bytes())
app=str(args.app/'Contents/MacOS'/info['CFBundleExecutable'])
def pids():
 out=subprocess.run(['/bin/ps','-axo','pid=,comm='],capture_output=True,text=True,check=True).stdout
 return sorted(int(x.strip().split(None,1)[0]) for x in out.splitlines() if len(x.strip().split(None,1))==2 and x.strip().split(None,1)[1]==app)
urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
before=pids(); result={'schema':1,'appVersion':info['CFBundleShortVersionString'],'bundleId':info['CFBundleIdentifier'],'existingPids':before,'isolatedProfile':True,'cdp':False,'injected':False}
with tempfile.TemporaryDirectory(prefix='code-codex-mac-probe-',dir='/private/tmp') as temp, ExitStack() as files:
 home=pathlib.Path(temp); (home/'profile').mkdir(); (home/'codex').mkdir(); (home/'workspace').mkdir(); (home/'workspace/Hello.txt').write_text('Isolated Mac parity fixture.\n')
 env=os.environ.copy(); env.update(HOME=temp,CODEX_HOME=str(home/'codex'))
 for k in list(env):
  if any(s in k.upper() for s in ['API_KEY','AUTH_TOKEN','ACCESS_TOKEN']): env.pop(k,None)
 with socket.socket() as s: s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
 log=files.enter_context((home/'launch.log').open('w+b'))
 p=OwnedGroup([app,'--user-data-dir='+str(home/'profile'),'--remote-debugging-address=127.0.0.1','--remote-debugging-port='+str(port),'--no-first-run','--disable-background-networking'],env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=log)
 result['probePid']=p.pid
 supervisor=None; supervisor_log=None
 try:
  deadline=time.monotonic()+30
  while time.monotonic()<deadline:
   if p.exited(): break
   owners=subprocess.run(['/usr/sbin/lsof','-nP','-a','-iTCP:'+str(port),'-sTCP:LISTEN','-Fp'],capture_output=True,text=True).stdout
   owner_pids=[int(x[1:]) for x in owners.splitlines() if x.startswith('p')]
   try:
    verified_ownership = bool(owner_pids) and all(os.getpgid(pid) == p.pid for pid in owner_pids)
   except ProcessLookupError:
    verified_ownership = False
   if verified_ownership:
    try:
     with urllib.request.urlopen('http://127.0.0.1:'+str(port)+'/json/version',timeout=1) as r: v=json.load(r)
     result['cdp']=True;result['browser']=v.get('Browser');result['protocol']=v.get('Protocol-Version');result['ownedListenerPids']=owner_pids
     with urllib.request.urlopen('http://127.0.0.1:'+str(port)+'/json/list',timeout=1) as r: targets=json.load(r)
     result['targets']=[{'type':t.get('type'),'url':t.get('url','').split('?')[0].split('#')[0]} for t in targets]
     page=next((t for t in targets if t.get('type')=='page' and t.get('url','').startswith('app://-/index.html')),None)
     if page:
      if not args.core: time.sleep(8)
      if args.core and supervisor is None:
       supervisor_log=files.enter_context((home/'supervisor.log').open('w+b'))
       supervisor=OwnedGroup([str(args.core),'attach','--port',str(port),'--workspace',str(home/'workspace'),'--allow-unsupported-version'],env=env,stdin=subprocess.DEVNULL,stdout=supervisor_log,stderr=supervisor_log)
       until=time.monotonic()+35
       while not supervisor.exited() and time.monotonic()<until:
        time.sleep(.25)
      probe=subprocess.run(['node',str(repo/'scripts/macos/probe-page.mjs')],input=json.dumps({'endpoint':page['webSocketDebuggerUrl']}),text=True,capture_output=True,timeout=8)
      if probe.returncode==0:
       result['domProbe']=json.loads(probe.stdout)
       result['injected']=result['domProbe'].get('explorerMounted',False)
      else: result['domProbe']={'error':'probe-execution-failed'}
      break
    except Exception as e: result['lastEndpointError']=type(e).__name__
   time.sleep(.25)
  result['stillRunningAtEnd']=not p.exited()
 finally:
  try:
   if supervisor is not None:
    result['supervisorExitCode']=supervisor.close()
  finally:
   exit_code=p.close()
   if result.get('stillRunningAtEnd') is False:
    result['exitCode']=exit_code
 if supervisor_log is not None:
  supervisor_log.seek(0); text=supervisor_log.read().decode('utf8','replace')
  result['supervisorSignals']={k:k in text for k in ['cdp_owner_verified','authorized_cdp_attach','explorer_supervisor_started','injected','qualified','incompatible','endpoint identity']}
  args.report.with_suffix('.supervisor.log').write_text(text)
 log.seek(0); b=log.read().decode('utf8','replace')
 result['logSignals']={k:k.lower() in b.lower() for k in ['DevTools listening','remote-debugging','Singleton','sandbox','error']}
result['existingPidsSurvived']=all(x in pids() for x in before)
result['status']='cdp-observed' if result['cdp'] else 'cdp-not-observed'
args.report.parent.mkdir(parents=True,exist_ok=True)
args.report.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))

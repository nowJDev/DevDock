// 브라우저 검증 화면에 격리된 코어의 읽기 전용 데이터를 제공합니다.
import {spawn} from 'node:child_process';
import {join} from 'node:path';
import type {Plugin} from 'vite';
export function previewPlugin(root:string):Plugin {
 return {name:'devdock-readonly-preview',configureServer(server){
  server.middlewares.use('/__devdock',async(req,res)=>{
   res.setHeader('Content-Type','application/json; charset=utf-8');
   if(req.method!=='POST'){res.statusCode=405;res.end(JSON.stringify({error:'POST 요청이 필요합니다.'}));return;}
   const origin=req.headers.origin;
   if(origin&&origin!=='http://'+req.headers.host){res.statusCode=403;res.end(JSON.stringify({error:'다른 출처의 요청을 허용하지 않습니다.'}));return;}
   let body='';for await(const chunk of req){body+=chunk;if(body.length>65536){res.statusCode=413;res.end(JSON.stringify({error:'요청이 너무 큽니다.'}));return;}}
   try {
    const params=JSON.parse(body||'{}');const command=(req.url??'').split('?')[0].replace(/^\//,'');
    if(command==='frontend_ready'){res.end(JSON.stringify({ok:true}));return;}
    const args=command==='get_snapshot'?['snapshot']:command==='get_registry'?['registry']:command==='get_history'?['history',String(params.monitorId??'A'),String(Math.max(1,Math.min(2160,Number(params.hours)||24)))]:command==='refresh_now'?['snapshot']:null;
    if(!args){res.statusCode=403;res.end(JSON.stringify({error:'읽기 전용 미리보기입니다. 편집과 근태 기능은 네이티브 앱에서 사용하세요.'}));return;}
    const exe=join(root,'src-tauri','target','debug','devdock-cli.exe');
    const child=spawn(exe,args,{cwd:root,windowsHide:true,env:{...process.env,DEVDOCK_DATA_DIR:join(root,'.devdock-data')},stdio:['ignore','pipe','pipe']});
    let output='';let failure='';const timer=setTimeout(()=>child.kill(),10000);
    child.stdout.on('data',part=>{if(output.length<1048576)output+=part;});child.stderr.on('data',part=>{if(failure.length<4096)failure+=part;});
    child.on('error',()=>{clearTimeout(timer);if(!res.writableEnded){res.statusCode=503;res.end(JSON.stringify({error:'미리보기 코어를 빌드해 주세요.'}));}});
    child.on('close',code=>{clearTimeout(timer);if(res.writableEnded)return;try{if(code!==0)throw new Error('코어 데이터를 읽지 못했습니다.');const result=JSON.parse(output);if(command==='get_snapshot')result.data.verificationMode=true;res.end(JSON.stringify(result.data));}catch(e){res.statusCode=503;res.end(JSON.stringify({error:String(e)}));}});
   }catch{res.statusCode=400;res.end(JSON.stringify({error:'요청 형식을 확인해 주세요.'}));}
  });
 }};
}

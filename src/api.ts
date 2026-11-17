// 화면의 명령을 네이티브 코어에 전달하고 브라우저 미연결 상태를 구분합니다.
import type {Api,Snapshot,Settings,Registry,History} from './contracts';
const native=()=>typeof window!=='undefined'&&'__TAURI_INTERNALS__' in window;
async function call<T>(command:string,args?:Record<string,unknown>):Promise<T>{
 if(native()){const {invoke}=await import('@tauri-apps/api/core');return invoke<T>(command,args);}
 if(import.meta.env.DEV){const response=await fetch('/__devdock/'+command,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(args??{})});if(!response.ok)throw new Error('네이티브 앱에서 사용할 수 있는 기능입니다.');const value=await response.json();if(value.error)throw new Error(value.error);return value;}
 throw new Error('네이티브 연결을 확인해 주세요.');
}
export const api:Api={
 ready:()=>call('frontend_ready'),snapshot:()=>call<Snapshot>('get_snapshot'),registry:()=>call<Registry>('get_registry'),
 mutate:(operation,expectedRevision)=>call('mutate_registry',{operation,expectedRevision}),
 saveSettings:(settings:Settings)=>call('save_settings',{settings}),
 history:(monitorId,hours)=>call<History>('get_history',{monitorId,hours}),
 attendance:action=>call('run_attendance',{action}),answerCheckout:(answer,promptDate)=>call('answer_checkout',{answer,promptDate}),
 setAutostart:enabled=>call('set_autostart',{enabled}),refresh:()=>call('refresh_now'),
 subscribe:async(name,callback)=>{if(!native())return()=>{};const {listen}=await import('@tauri-apps/api/event');return listen(name,event=>callback(event.payload));}
};

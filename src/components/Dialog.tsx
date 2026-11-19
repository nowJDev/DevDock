// 문서와 보고 편집 창의 키보드 포커스와 닫기 후 복원을 제공한다.
import {useLayoutEffect,useRef,type ReactNode} from 'react';
import {X} from 'lucide-react';
function tabStops(dialog:HTMLElement){
 const closed=Array.from(dialog.querySelectorAll('details:not([open])'));
 return Array.from(dialog.querySelectorAll<HTMLElement>('button,input:not([type="hidden"]),select,textarea,summary,a[href],[tabindex]')).filter(element=>
  !element.matches(':disabled')&&!element.closest('[hidden]')&&(element.tabIndex>=0||element.tagName==='SUMMARY')&&
  closed.every(details=>!details.contains(element)||details.querySelector(':scope > summary')?.contains(element)));
}
export default function Dialog({title,children,onClose,returnFocus,wide=false,className=''}:{title:string;children:ReactNode;onClose:()=>void;returnFocus:HTMLElement|null;wide?:boolean;className?:string}){
 const ref=useRef<HTMLDivElement>(null),close=useRef(onClose);close.current=onClose;
 useLayoutEffect(()=>{
  const dialog=ref.current;
  if(dialog&&!dialog.contains(document.activeElement))tabStops(dialog)[0]?.focus();
  const handler=(e:KeyboardEvent)=>{
   const dialogs=document.querySelectorAll('.modal-shade [role="dialog"]');if(dialog!==dialogs[dialogs.length-1])return;
   if(e.key==='Escape'){e.preventDefault();close.current();return;}if(e.key!=='Tab'||!dialog)return;
   const elements=tabStops(dialog);if(!elements.length)return;
   const first=elements[0],last=elements[elements.length-1];
   if(e.shiftKey&&(document.activeElement===first||!dialog.contains(document.activeElement))){e.preventDefault();last.focus();}else if(!e.shiftKey&&(document.activeElement===last||!dialog.contains(document.activeElement))){e.preventDefault();first.focus();}
  };
  document.addEventListener('keydown',handler);return()=>{document.removeEventListener('keydown',handler);queueMicrotask(()=>{if(dialog?.isConnected||!returnFocus?.isConnected||document.activeElement!==document.body)return;const dialogs=document.querySelectorAll('.modal-shade [role="dialog"]');const top=dialogs[dialogs.length-1];if(!top||top.contains(returnFocus))returnFocus.focus();});};
 },[returnFocus]);
 return <div className="modal-shade"><div ref={ref} className={['modal',wide?'modal-wide':'',className].filter(Boolean).join(' ')} role="dialog" aria-modal="true" aria-label={title}><header><h2>{title}</h2><button type="button" className="icon-button" aria-label="닫기" onClick={onClose}><X size={18}/></button></header>{children}</div></div>;
}

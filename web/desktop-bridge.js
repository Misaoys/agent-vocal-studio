(()=>{
  'use strict';
  let next=1;
  const pending=new Map();
  window.__desktopReply=(id,result)=>{
    const request=pending.get(id);
    if(!request)return;
    pending.delete(id);
    if(result.ok)request.resolve(result.data);
    else request.reject(new Error(result.error));
  };
  const bridge=Object.freeze({
    invoke(op,args={}){
      return new Promise((resolve,reject)=>{
        const id=next++;
        pending.set(id,{resolve,reject});
        try{window.ipc.postMessage(JSON.stringify({id,op,args}))}
        catch(error){pending.delete(id);reject(error)}
      });
    }
  });
  Object.defineProperty(window,'desktopNative',{value:bridge});
})();

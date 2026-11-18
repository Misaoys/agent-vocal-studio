'use strict';

const filePanel=document.createElement('dialog');
filePanel.className='studio-files';
filePanel.setAttribute('aria-labelledby','studioFileHeading');
filePanel.innerHTML=`
  <div class="dialog-heading"><h2 id="studioFileHeading"></h2><button id="studioFileClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="file-location"><button id="studioFileParent" aria-label="上一级">${icon('up')}</button>
    <input id="studioFilePath" aria-label="目录路径" spellcheck="false"><button id="studioFileGo" aria-label="前往目录">${icon('right')}</button></div>
  <div class="file-list-layout"><nav id="studioFilePlaces" aria-label="常用位置"></nav><div id="studioFileEntries" role="listbox" aria-label="当前目录" aria-multiselectable="true"></div></div>
  <div class="file-pagination"><button id="studioFileBack" aria-label="上一页">${icon('left')}</button><output id="studioFilePage"></output><button id="studioFileNext" aria-label="下一页">${icon('right')}</button></div>
  <div class="file-bottom"><output id="studioFileStatus" role="status"></output><input id="studioFileName" aria-label="保存文件名" hidden>
    <button id="studioFileApply" class="primary"></button></div>`;
document.body.append(filePanel);
wireDialog(filePanel);

const confirmPanel=document.createElement('dialog');
confirmPanel.className='studio-confirm';
confirmPanel.setAttribute('aria-labelledby','studioConfirmTitle');
confirmPanel.innerHTML=`<form method="dialog"><div class="dialog-heading"><h2 id="studioConfirmTitle"></h2></div><p id="studioConfirmText"></p>
  <div id="ppsfReferences" hidden>
    <div class="ppsf-reference"><button type="button" data-ppsf-role="vocal_path">选择原版人声</button><button type="button" data-clear-ppsf="vocal_path" aria-label="移除原版人声" hidden>${icon('close')}</button></div>
    <div class="ppsf-reference"><button type="button" data-ppsf-role="backing_path">选择伴奏</button><button type="button" data-clear-ppsf="backing_path" aria-label="移除伴奏" hidden>${icon('close')}</button></div>
    <small>使用从工程起点导出的分轨</small>
  </div>
  <div class="arrange-dialog-footer"><button value="no">取消</button><button value="yes" id="studioConfirmYes" class="primary"></button></div></form>`;
document.body.append(confirmPanel);
wireDialog(confirmPanel);

function studioConfirm(title,message,action){
  if(confirmPanel.open)return Promise.resolve(false);
  $('studioConfirmTitle').textContent=title;
  $('studioConfirmText').textContent=message;
  $('studioConfirmYes').textContent=action;
  confirmPanel.returnValue='no';
  const answer=new Promise(resolve=>confirmPanel.addEventListener('close',()=>resolve(confirmPanel.returnValue==='yes'),{once:true}));
  confirmPanel.showModal();
  return answer;
}

const ppsfReferences=$('ppsfReferences');
function choosePpsfReferences(target,directory){
  ppsfReferences.hidden=false;
  for(const button of ppsfReferences.querySelectorAll('[data-ppsf-role]')){
    const role=button.dataset.ppsfRole;
    const label=role==='vocal_path'?'原版人声':'伴奏';
    const clear=ppsfReferences.querySelector(`[data-clear-ppsf="${role}"]`);
    const update=()=>{
      const file=target[role]?.split(/[\\/]/).at(-1);
      button.textContent=file||'选择'+label;
      button.title=target[role]||'';
      button.setAttribute('aria-label',file?label+' · '+file:'选择'+label);
      clear.hidden=!file;
    };
    button.onclick=async()=>{
      const files=await openStudioFiles({extension:'audio',single:true,directory,heading:'选择'+label});
      if(files?.length&&confirmPanel.open){target[role]=files[0];update()}
    };
    clear.onclick=()=>{delete target[role];update();button.focus()};
    update();
  }
}

let ppsfImportPending=false;
async function importScore(args,filename){
  if(!/\.ppsf$/i.test(filename))return api('project_import',args);
  if(ppsfImportPending||confirmPanel.open)return null;
  ppsfImportPending=true;
  try{
    const {migration}=await api('project_import',{...args,dry_run:true});
    const {track_count,note_count,bpm,meter,omitted_audio_tracks,omitted_vocaloid_tracks,source_sha256}=migration;
    const omittedTracks=[
      omitted_audio_tracks&&`${omitted_audio_tracks} 条音频轨`,
      omitted_vocaloid_tracks&&`${omitted_vocaloid_tracks} 条旧版 VOCALOID 轨`
    ].filter(Boolean);
    const lines=[
      filename,
      `${track_count} 条歌声轨 · ${note_count} 个音符 · ${bpm} BPM · ${meter.join('/')}`,
      '保留歌词。Piapro 音色、调声、发音设置和混音需重配。',
      ...(omittedTracks.length?[`需单独处理 ${omittedTracks.join('、')}。`]:[])
    ];
    if(confirmPanel.open)return null;
    const reference_audio={};
    if(window.desktopNative)choosePpsfReferences(reference_audio,args.path?.replace(/[^\\/]+$/,''));
    const approved=await studioConfirm('导入 PPSF 曲谱',lines.join('\n'),'导入副本');
    ppsfReferences.hidden=true;
    if(!approved)return null;
    const request={...args,expected_source_sha256:source_sha256};
    if(Object.keys(reference_audio).length){
      request.reference_audio=reference_audio;
      toast('正在导入曲谱和分轨…');
    }
    return await api('project_import',request);
  }finally{ppsfImportPending=false}
}

async function showStudioAbout(){
  const version=window.desktopNative?(await window.desktopNative.invoke('host_info')).version:(await api('studio_status')).version;
  await studioConfirm('Agent Vocal Studio '+version,'本地歌声编辑器 · Rust · MCP','完成');
}

let fileRequest=null,fileListing=null,fileEpoch=0;
const fileSelected=new Set();
const callDesktop=(operation,args)=>window.desktopNative.invoke(operation,args);

const arrangeTransportLine=document.createElement('i');
const scoreTransportLine=document.createElement('i');
for(const line of [arrangeTransportLine,scoreTransportLine]){
  line.className='studio-playhead';
  line.setAttribute('aria-hidden','true');
}
arrangeViewport.append(arrangeTransportLine);
const scoreViewport=$('rollViewport');
scoreViewport.append(scoreTransportLine);

let followPlayback=true;
try{followPlayback=localStorage.getItem('avs-follow-playback')!=='off'}catch{}
const followPlaybackButton=document.createElement('button');
followPlaybackButton.id='followPlayback';
followPlaybackButton.type='button';
followPlaybackButton.innerHTML=icon('follow');
followPlaybackButton.title='跟随播放游标';
followPlaybackButton.setAttribute('aria-label','跟随播放游标');
$('loop').after(followPlaybackButton);
const manualScrollUntil=new WeakMap(),followedScrollLeft=new WeakMap();
for(const view of [arrangeViewport,scoreViewport]){
  const deferFollow=()=>manualScrollUntil.set(view,performance.now()+1600);
  for(const type of ['wheel','pointerdown'])
    view.addEventListener(type,event=>{if(event.isTrusted)deferFollow()},{capture:true,passive:true});
  view.addEventListener('scroll',()=>{
    if(followedScrollLeft.get(view)!==view.scrollLeft)deferFollow();
    followedScrollLeft.delete(view);
  },{passive:true});
}

function updateFollowButton(){
  followPlaybackButton.setAttribute('aria-pressed',String(followPlayback));
}
followPlaybackButton.onclick=()=>{
  followPlayback=!followPlayback;
  try{localStorage.setItem('avs-follow-playback',followPlayback?'on':'off')}catch{}
  updateFollowButton();
  if(followPlayback){
    manualScrollUntil.delete(arrangeViewport);
    manualScrollUntil.delete(scoreViewport);
    updatePlaybackView();
  }
};
updateFollowButton();

function shouldFollow(view){
  return followPlayback&&!audio.paused&&view.clientWidth>0&&
    performance.now()>=(manualScrollUntil.get(view)??0);
}
function followHorizontal(view,position,inset){
  const width=view.clientWidth-inset;
  if(width<=0)return;
  const current=view.scrollLeft;
  if(position<current+inset||position>current+inset+width*.82){
    const target=Math.max(0,Math.min(view.scrollWidth-view.clientWidth,position-inset-width*.24));
    if(Math.abs(target-current)>1){
      view.scrollLeft=target;
      followedScrollLeft.set(view,view.scrollLeft);
    }
  }
}
function updateArrangePlayhead(){
  if(!project)return;
  const beat=audio.paused&&!audio.currentTime?cursorBeat:secondsBeat(audio.currentTime);
  const x=arrangeLabelWidth+beat*arrangeZoom,y=arrangeHeader+arrangeViewport.scrollTop;
  const relative=x-arrangeViewport.scrollLeft;
  arrangeTransportLine.hidden=relative<arrangeLabelWidth||relative>arrangeViewport.clientWidth;
  arrangeTransportLine.style.transform='translate3d('+x+'px,'+y+'px,0)';
  arrangeTransportLine.style.height=Math.max(0,arrangeViewport.clientHeight-arrangeHeader)+'px';
}
function updatePlaybackView(){
  scoreTransportLine.hidden=!project||audio.paused&&!audio.currentTime;
  if(!project)return;
  const beat=secondsBeat(audio.currentTime),x=beatX(beat);
  if(arrangementVisible&&shouldFollow(arrangeViewport)&&!arrangePointer&&!clipDrag&&!trackOrderDrag)
    followHorizontal(arrangeViewport,arrangeLabelWidth+beat*arrangeZoom,arrangeLabelWidth);
  if(shouldFollow(scoreViewport)&&!drag&&!curveDrag){
    const width=scoreViewport.clientWidth;
    const ahead=(width-keyW)/zoom;
    // Give short or empty parts enough timeline to follow playback of other tracks.
    if(ahead>0&&x+width*.25>roll.clientWidth){
      const end=Math.min(arrangementModel().limit+16,Math.ceil((beat+ahead*2)/4)*4);
      if(end>beats()){
        playbackRollEnd=end;
        draw();
      }
    }
    followHorizontal(scoreViewport,x,keyW);
  }
  scoreTransportLine.style.transform='translate3d('+x+'px,0,0)';
  scoreTransportLine.style.height=roll.clientHeight+'px';
  updateArrangePlayhead();
}
for(const event of ['play','timeupdate','pause','ended'])audio.addEventListener(event,updatePlaybackView);
document.fonts.ready.then(()=>{draw();queueArrangementDraw()});

function fileSelectionChanged(){
  $('studioFileApply').disabled=!fileListing||(!fileRequest?.url&&!fileSelected.size);
  for(const entry of $('studioFileEntries').querySelectorAll('[data-path]')){
    entry.setAttribute('aria-selected',String(fileSelected.has(entry.dataset.path)));
  }
}

async function listStudioFiles(directory,page=0){
  const request=fileRequest,epoch=++fileEpoch;
  if(!request)return;
  fileListing=null;
  $('studioFileStatus').textContent='正在读取…';
  $('studioFileApply').disabled=true;
  try{
    const listing=await callDesktop('file_browse',{directory,page,extension:request.extension});
    if(epoch!==fileEpoch||fileRequest!==request)return;
    fileListing=listing;
    fileSelected.clear();
    $('studioFilePath').value=listing.directory;
    $('studioFileParent').disabled=!listing.parent;
    $('studioFileBack').disabled=listing.page===0;
    $('studioFileNext').disabled=listing.page+1>=listing.pages;
    $('studioFilePage').textContent=listing.pages>1?`${listing.page+1} / ${listing.pages}`:'';
    $('studioFilePlaces').replaceChildren(...listing.locations.map(location=>{
      const button=document.createElement('button');
      button.innerHTML=icon('folder')+'<span></span>';
      button.lastElementChild.textContent=location.label;
      button.onclick=()=>void listStudioFiles(location.path);
      return button;
    }));
    const list=$('studioFileEntries');
    list.replaceChildren(...listing.entries.map(item=>{
      const button=document.createElement('button');
      button.type='button';
      button.setAttribute('role','option');
      button.dataset.path=item.path;
      button.dataset.directory=String(item.directory);
      button.innerHTML=icon(item.directory?'folder':'wave')+'<span></span>';
      button.lastElementChild.textContent=item.name;
      button.onclick=event=>{
        if(item.directory){void listStudioFiles(item.path);return}
        if(request.url){$('studioFileName').value=item.name;return}
        if(request.single||(!event.ctrlKey&&!event.metaKey))fileSelected.clear();
        if(fileSelected.has(item.path))fileSelected.delete(item.path);else fileSelected.add(item.path);
        fileSelectionChanged();
      };
      button.ondblclick=()=>{if(!item.directory&&!request.url){if(request.single)fileSelected.clear();fileSelected.add(item.path);fileRequest.value=[...fileSelected];filePanel.close()}};
      return button;
    }));
    list.scrollTop=0;
    $('studioFileStatus').textContent=listing.entries.length?'':'此目录没有支持的文件';
    fileSelectionChanged();
  }catch(error){
    if(epoch===fileEpoch&&fileRequest===request){fileListing=null;$('studioFileStatus').textContent=error.message;fileSelectionChanged()}
  }
}

function openStudioFiles(options){
  if(filePanel.open)return Promise.resolve(null);
  fileRequest={...options,value:null};
  fileListing=null;fileSelected.clear();
  $('studioFileEntries').setAttribute('aria-multiselectable',String(!options.single));
  $('studioFileHeading').textContent=options.heading||(options.url?'保存导出':'导入文件');
  $('studioFileName').hidden=!options.url;
  $('studioFileName').value=options.name||'';
  $('studioFileApply').textContent=options.url?'保存':options.single?'选择':'导入';
  const done=new Promise(resolve=>filePanel.addEventListener('close',()=>{
    const outcome=fileRequest?.value;
    if(outcome&&fileListing)try{localStorage.setItem('studio-file-directory',fileListing.directory)}catch{}
    fileEpoch++;fileRequest=null;resolve(outcome);
  },{once:true}));
  filePanel.showModal();
  void listStudioFiles(options.directory||localStorage.getItem('studio-file-directory')||undefined);
  return done;
}

function studioChoose(extension){return openStudioFiles({extension})}
async function studioSave(url){
  const info=await callDesktop('export_name',{url});
  return openStudioFiles({url,name:info.name,extension:info.name.split('.').at(-1)});
}

$('studioFileClose').onclick=()=>filePanel.close();
$('studioFileGo').onclick=()=>void listStudioFiles($('studioFilePath').value.trim());
$('studioFileParent').onclick=()=>{if(fileListing?.parent)void listStudioFiles(fileListing.parent)};
$('studioFileBack').onclick=()=>void listStudioFiles(fileListing.directory,fileListing.page-1);
$('studioFileNext').onclick=()=>void listStudioFiles(fileListing.directory,fileListing.page+1);
$('studioFilePath').onkeydown=event=>{if(event.key==='Enter'&&!event.isComposing){event.preventDefault();$('studioFileGo').click()}};
$('studioFileName').onkeydown=event=>{if(event.key==='Enter'&&!event.isComposing){event.preventDefault();$('studioFileApply').click()}};
$('studioFileApply').onclick=async()=>{
  const request=fileRequest;
  if(!request||!fileListing)return;
  if(!request.url){if(fileSelected.size){request.value=[...fileSelected];filePanel.close()}return}
  const args={url:request.url,directory:fileListing.directory,filename:$('studioFileName').value.trim()};
  $('studioFileApply').disabled=true;
  try{
    let saved=await callDesktop('save_export',args);
    if(saved.occupied){
      if(!await studioConfirm('同名文件已存在',args.filename,'替换'))return;
      if(request!==fileRequest)return;
      saved=await callDesktop('save_export',{...args,replace:true});
    }
    if(request===fileRequest){request.value=saved;filePanel.close()}
  }catch(error){if(request===fileRequest)$('studioFileStatus').textContent=error.message}
  finally{if(request===fileRequest)fileSelectionChanged()}
};

if(window.desktopNative){
  const topbar=document.querySelector('.topbar');
  const brand=document.createElement('button');
  brand.id='studioMenu';brand.title='工具 (Ctrl+K)';brand.setAttribute('aria-label','工具');
  brand.innerHTML=icon('grid')+'<span>工具</span>';
  brand.onclick=()=>window.EditorCommands.execute('palette');
  topbar.prepend(brand);
  const controls=document.createElement('div');
  controls.className='studio-window-controls';
  controls.innerHTML=`<button id="studioMinimize" aria-label="最小化">${icon('minus')}</button><button id="studioMaximize" aria-label="最大化">${icon('grid')}</button><button id="studioClose" aria-label="关闭窗口">${icon('close')}</button>`;
  topbar.append(controls);
  const grip=document.createElement('div');
  grip.id='studioDrag';
  grip.title='拖动窗口 · 双击最大化或还原';
  grip.innerHTML='<svg class="ui-icon" viewBox="0 0 20 20" aria-hidden="true"><path d="M6 7h8M6 10h8M6 13h8" fill="none" stroke="currentColor" stroke-linecap="round"/></svg>';
  $('saveState').after(grip);
  function syncWindow(status){
    document.documentElement.dataset.maximized=String(status.maximized);
    $('studioMaximize').setAttribute('aria-label',status.maximized?'还原窗口':'最大化');
  }
  function control(action,edge){void callDesktop('window_control',{action,edge}).then(syncWindow).catch(error=>toast(error.message,true))}
  $('studioMinimize').onclick=()=>control('minimize');
  $('studioMaximize').onclick=()=>control('maximize');
  $('studioClose').onclick=()=>window.desktopRequestClose();
  let movingWindow=null;
  let dragFrame=0;
  function applyWindowDrag(){
    dragFrame=0;
    if(movingWindow?.delta)void callDesktop('window_control',{action:'move_drag',...movingWindow.delta}).catch(error=>toast(error.message,true));
  }
  topbar.addEventListener('pointerdown',event=>{
    if(event.button!==0||event.target.closest('button,input,select,textarea')||document.documentElement.dataset.maximized==='true')return;
    movingWindow={id:event.pointerId,x:event.screenX,y:event.screenY,delta:null};
    try{topbar.setPointerCapture(event.pointerId)}catch(error){if(error.name!=='NotFoundError')throw error}
    void callDesktop('window_control',{action:'move_start'}).catch(error=>toast(error.message,true));
  });
  topbar.addEventListener('pointermove',event=>{
    if(movingWindow?.id!==event.pointerId)return;
    movingWindow.delta={dx:event.screenX-movingWindow.x,dy:event.screenY-movingWindow.y};
    if(!dragFrame)dragFrame=requestAnimationFrame(applyWindowDrag);
  });
  for(const kind of ['pointerup','pointercancel','lostpointercapture'])topbar.addEventListener(kind,event=>{
    if(movingWindow?.id!==event.pointerId)return;
    if(dragFrame){cancelAnimationFrame(dragFrame);applyWindowDrag()}
    movingWindow=null;
    void callDesktop('window_control',{action:'move_end'});
  });
  topbar.addEventListener('dblclick',event=>{
    if(!event.target.closest('button,input,select,textarea'))control('maximize');
  });
  for(const edge of ['n','s','e','w','ne','nw','se','sw']){
    const handle=document.createElement('div');
    handle.className='studio-resize';handle.dataset.edge=edge;handle.setAttribute('aria-hidden','true');
    handle.onpointerdown=event=>{if(event.button===0){event.preventDefault();control('resize',edge)}};
    document.body.append(handle);
  }
  control('state');
}

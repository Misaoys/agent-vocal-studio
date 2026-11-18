'use strict';

let clipSelection=null,clipDrag=null,clipBusy=false,clipDraft=null,clipPreview=null;
let groupedClips=[];
let audioGroupClipboard=null;
const songActions=arrangementPanel.querySelector('.arrange-actions');
const clipActions=document.createElement('div');
clipActions.className='arrange-actions';
clipActions.hidden=true;
clipActions.innerHTML=`
  <button id="clipSplit" title="预览切点 (Shift+S)">${icon('scissors')} 切分</button>
  <button id="clipCopy" title="复制片段">${icon('copy')} 复制</button>
  <button id="clipSettings" title="片段增益、淡化与范围">${icon('grid')} 片段</button>
  <button id="clipSlip" title="滑移片段内容 (Alt+S)">${icon('slip')} 滑移</button>
  <button id="clipTempo" title="跟随工程速度">${icon('wave')} 跟速</button>
  <button id="clipWarp" title="把片段内的波形位置对齐到指定拍位">${icon('grid')} 对拍</button>`;
songActions.after(clipActions);
const appendClipButton=document.createElement('button');
appendClipButton.id='clipAppend';
appendClipButton.title='向当前音频轨添加音频';
appendClipButton.innerHTML=icon('plus')+' 音频';
appendClipButton.hidden=true;
clipActions.after(appendClipButton);
const groupActions=document.createElement('div');
groupActions.className='arrange-actions';groupActions.hidden=true;
groupActions.innerHTML=`<button id="groupSplit" title="预览所选音频切点 (Shift+S)">${icon('scissors')} 切分</button><button id="groupCopy" title="复制所选音频 (Ctrl+D)">${icon('copy')} 复制</button><button id="groupSlip" title="滑移所选片段内容 (Alt+S)">${icon('slip')} 滑移</button><button id="groupCrossfade" title="连接同一音轨上的两个片段" hidden>${icon('wave')} 交叉淡化</button><button id="groupTempo" title="设置所选片段的跟速模式">${icon('wave')} 跟速</button><button id="groupRemove" title="移除所选音频 (Delete)">${icon('close')} 移除</button>`;
clipActions.after(groupActions);
appendClipButton.onclick=async()=>{
  if(!window.desktopNative||!isAudioTrack(tr()))return;
  const target={project_id:project.id,track_id:tr().id,start:cursorBeat};
  try{
    const paths=await studioChoose('audio');
    await importNativePaths(paths,target);
  }catch(error){toast(error.message,true)}
};

const clipDialog=document.createElement('dialog');
clipDialog.id='clipDialog';
clipDialog.className='arrange-dialog';
clipDialog.innerHTML=`
  <div class="dialog-heading"><h2>音频片段</h2><button id="clipClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>操作<select id="clipKind">
      <option value="set">属性</option><option value="split">切分</option><option value="move">移动</option>
      <option value="copy">复制</option><option value="trim">裁剪</option><option value="remove">移除</option>
    </select></label>
    <output id="clipLength"></output>
  </div>
  <div id="clipFields" data-kind="set">
    <div data-clip-fields="set"><label>名称<input id="clipName" maxlength="100"></label>
      <label>增益 · dB<input id="clipGain" type="number" min="-60" max="12" step="0.1"></label>
      <label><span id="clipFadeInUnit">淡入 · ms</span><input id="clipFadeIn" type="number" min="0" step="any"></label>
      <label><span id="clipFadeOutUnit">淡出 · ms</span><input id="clipFadeOut" type="number" min="0" step="any"></label></div>
    <div data-clip-fields="split"><label>切分点 · 拍<input id="clipAt" type="number" min="0" step="any"></label></div>
    <div data-clip-fields="move copy"><label>放置拍<input id="clipTargetBeat" type="number" min="0" step="any"></label>
      <label>音频轨<select id="clipTargetTrack"></select></label><label class="clip-check"><input id="clipAllowOverlap" type="checkbox"> 允许叠放</label></div>
    <div data-clip-fields="trim"><label title="以原片段的源音频起点为零">起点偏移 · 秒<input id="clipFirst" type="number" step="any"></label>
      <label title="以原片段的源音频起点为零">终点位置 · 秒<input id="clipEnd" type="number" min="0" step="any"></label>
      <label class="clip-check"><input id="clipTrimOverlap" type="checkbox"> 允许延长时叠放</label></div>
  </div>
  <div id="clipPreviewInfo" role="status"></div>
  <div class="arrange-dialog-footer"><button id="clipPreview" class="ghost">预览</button><button id="clipApply" class="primary">应用</button></div>`;
document.body.append(clipDialog);
wireDialog(clipDialog);
$('clipClose').onclick=()=>clipDialog.close();

const crossfadeDialog=document.createElement('dialog');
crossfadeDialog.id='crossfadeDialog';
crossfadeDialog.className='arrange-dialog';
crossfadeDialog.innerHTML=`
  <div class="dialog-heading"><h2>交叉淡化</h2><button id="crossfadeClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>时长 · ms<input id="crossfadeLength" type="number" min="0.042" step="any" placeholder="自动"></label>
    <label>曲线<select id="crossfadeCurve"><option value="">自动</option><option value="linear">线性</option><option value="equal_power">等功率</option></select></label>
  </div>
  <svg id="crossfadePlot" viewBox="0 0 320 60" role="img" aria-label="淡出和淡入的增益曲线" hidden>
    <polyline id="crossfadeOut"/><polyline id="crossfadeIn"/>
  </svg>
  <output id="crossfadeStatus" role="status"></output>
  <div class="arrange-dialog-footer"><button id="previewCrossfade">预览</button><button class="primary" id="applyCrossfade" disabled>应用</button></div>`;
document.body.append(crossfadeDialog);
wireDialog(crossfadeDialog);
const crossfadeAudition=arrangementAuditionControls(crossfadeDialog);
$('crossfadeClose').onclick=()=>crossfadeDialog.close();
let crossfadeTarget=null,crossfadeDraft=null;

const splitDialog=document.createElement('dialog');
splitDialog.id='splitDialog';
splitDialog.className='arrange-dialog';
splitDialog.setAttribute('aria-labelledby','splitTitle');
splitDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="splitTitle">切点试听</h2><button id="splitClose" aria-label="关闭">${icon('close')}</button></div>
  <output id="splitStatus" role="status"></output>
  <div class="arrange-dialog-footer"><button class="primary" id="applySplit" disabled>应用切分</button></div>`;
document.body.append(splitDialog);
wireDialog(splitDialog);
const splitAudition=arrangementAuditionControls(splitDialog);
let splitReview=null;
$('splitClose').onclick=()=>splitDialog.close();
splitDialog.addEventListener('close',()=>{splitReview=null});

function selectedAudioClip(){
  if(!clipSelection||!project||clipSelection.projectId!==project.id)return null;
  const track=project.tracks.find(t=>t.id===clipSelection.trackId);
  const clip=trackClips(track).find(clip=>clip.id===clipSelection.clipId);
  return clip?{track,clip}:null;
}

function clipGroupItems(){
  if(!project)return [];
  return groupedClips.filter(ref=>ref.projectId===project.id).flatMap(ref=>{
    const track=project.tracks.find(item=>item.id===ref.track_id);
    const clip=trackClips(track).find(item=>item.id===ref.clip_id);
    return clip?[{track,clip}]:[];
  });
}

function clipSelectionRefs(){
  const one=selectedAudioClip(),group=clipGroupItems();
  return (group.length?group:one?[one]:[]).map(({track,clip})=>({track_id:track.id,clip_id:clip.id}));
}

function setClipGroup(items){
  groupedClips=items.length>1?items.map(({track,clip})=>({projectId:project.id,track_id:track.id,clip_id:clip.id})):[];
  clipSelection=items.length===1?{projectId:project.id,trackId:items[0].track.id,clipId:items[0].clip.id}:null;
  if(items.length){trackId=items[0].track.id;selected=null;chooseGroup([])}
  refresh();
}

function selectClipsInRange(range){
  const tracks=new Set(range.ids);
  setClipGroup(project.tracks.filter(track=>tracks.has(track.id)).flatMap(track=>trackClips(track)
    .filter(clip=>clip.start<range.end&&clipEndBeat(clip)>range.start).map(clip=>({track,clip}))));
}

function showClipTools(){
  const selected=arrangementVisible?selectedAudioClip():null;
  const group=clipGroupItems(),count=group.length;
  const rangeMode=arrangementVisible&&arrangeRangeTouchesNotes();
  songActions.hidden=!rangeMode&&(!!selected||count>1);
  clipActions.hidden=rangeMode||!selected;
  groupActions.hidden=rangeMode||count<2;
  $('groupTakes')?.toggleAttribute('hidden',!arrangementVisible||count!==2||
    group.some(item=>item.clip.beat_length));
  $('groupCrossfade').hidden=!arrangementVisible||count!==2||group[0].track.id!==group[1].track.id;
  appendClipButton.hidden=!(arrangementVisible&&isAudioTrack(tr())&&window.desktopNative);
  if(rangeMode){
    paintArrangeRangeLabel();
  }else if(count>1){
    const tracks=new Set(group.map(item=>item.track.id));
    $('arrangeSpan').textContent=`${count} 个片段 · ${tracks.size} 轨`;
  }else if(selected){
    $('arrangeSpan').textContent=selected.clip.name+' · '+((clipEndFrame(selected.clip)-clipStartFrame(selected.clip))/clipRate).toFixed(3)+' 秒';
  }else if(!arrangeSelection){
    $('arrangeSpan').textContent='';
  }
  queueArrangementDraw();
}

function selectAudioClip(track,clip){
  groupedClips=[];
  clipSelection=clip?{projectId:project.id,trackId:track.id,clipId:clip.id}:null;
  if(track)trackId=track.id;
  if(clip)updateArrangeSelection(null);
  workspaceFocus='arrangement';
  refresh();
  showClipTools();
}

function paintClipSelection(c,track,clip,x,top){
  const single=clipSelection?.projectId===project.id&&clipSelection?.trackId===track.id&&clipSelection.clipId===clip.id;
  const group=groupedClips.some(ref=>ref.projectId===project.id&&ref.track_id===track.id&&ref.clip_id===clip.id);
  if(!single&&!group)return;
  const left=x(clip.start),right=x(clipEndBeat(clip)),width=Math.max(2,right-left);
  c.fillStyle='#f2f2f216';c.fillRect(left,top+8,width,38);
  c.strokeStyle='#e1e1e3';c.lineWidth=1.25;c.strokeRect(left+.5,top+8.5,width-1,37);
  if(single&&width>20){c.fillStyle='#d0d0d3';c.fillRect(left+3,top+23,2,11);c.fillRect(right-5,top+23,2,11)}
}

function paintClipDrag(c,x,y){
  if(!clipDrag?.placement)return;
  for(const {start,end,row} of clipDrag.placement.items||[clipDrag.placement]){
    const top=y(row);
    c.fillStyle='#d0d0d032';c.fillRect(x(start),top+8,Math.max(2,x(end)-x(start)),38);
    c.strokeStyle='#c1c1c4';c.strokeRect(x(start),top+8,Math.max(2,x(end)-x(start)),38);
  }
}

function pickAudioClipAt(point){
  if(point.x<arrangeLabelWidth||point.y<arrangeHeader)return null;
  const track=project.tracks[point.row];
  if(!isAudioTrack(track))return null;
  const clip=trackClips(track).findLast(c=>point.beat>=c.start-1e-8&&point.beat<=clipEndBeat(c)+1e-8);
  return clip?{track,clip}:null;
}

function clipRequest(){
  const selected=selectedAudioClip();
  if(!selected)throw new Error('先选择音频片段');
  return {project_id:project.id,expected_revision:project.revision,track_id:selected.track.id,clip_id:selected.clip.id};
}

function clipError(message){
  if(message.includes('CLIP_OVERLAP'))return '目标位置已有片段。可移到空位，或在片段面板允许叠放。';
  if(message.includes('SPLIT_IN_FADE'))return '切分点位于淡入或淡出范围内。请选择淡化范围之外的切分点。';
  if(message.includes('REVISION_CONFLICT'))return '工程已更新。请重新选择片段。';
  if(message.includes('Split must land'))return '切分点必须位于片段内部。';
  return message;
}

async function applyClipEdit(request,previewId){
  const task=editChain.catch(()=>{}).then(()=>api('audio_clip_edit',{...request,preview_id:previewId}));
  editChain=task;
  const result=await task;
  if(project?.id!==request.project_id)return result;
  audio.pause();
  const selected=result.after.at(-1);
  if(selected){trackId=selected.track_id;clipSelection={projectId:project.id,trackId:selected.track_id,clipId:selected.clip.id}}
  else clipSelection=null;
  accept(result.project);
  toast({split:'已切分',move:'已移动',copy:'已复制',trim:'已裁剪',set:'已保存片段属性',remove:'已移除片段'}[result.action]);
  refreshList().catch(()=>{});
  return result;
}

async function quickClipAction(request){
  if(clipBusy)return;
  clipBusy=true;
  try{
    await editChain.catch(()=>{});
    const preview=await api('audio_clip_edit',request);
    await applyClipEdit(request,preview.preview_id);
  }catch(error){toast(clipError(error.message),true)}
  finally{clipBusy=false}
}

function groupRequest(action){
  return {project_id:project.id,expected_revision:project.revision,clips:clipSelectionRefs(),action};
}

async function quickGroupAction(request){
  if(clipBusy||!request.clips.length)return;
  clipBusy=true;
  try{
    await editChain.catch(()=>{});
    const preview=await api('audio_clip_group',request);
    await applyClipGroup(request,preview.preview_id);
  }catch(error){toast(clipError(error.message),true)}
  finally{clipBusy=false}
}

function bindClipAudition(player,request,preview_id){
  const {project_id,expected_revision,clips,action}=request;
  player.set({scope:{project_id,expected_revision},preview_id,edit:{kind:'clip_group',edit:{clips,action}}});
}

async function applyClipGroup(request,previewId,listening=null){
  const task=editChain.catch(()=>{}).then(()=>listening
    ?api('arrangement_audition',listening):api('audio_clip_group',{...request,preview_id:previewId}));
  editChain=task;
  const result=await task;
  if(project?.id!==request.project_id)return;
  audio.pause();
  clipSelection=null;
  groupedClips=result.after.map(item=>({projectId:project.id,track_id:item.track_id,clip_id:item.clip.id}));
  if(groupedClips.length===1){const first=groupedClips.pop();clipSelection={projectId:project.id,trackId:first.track_id,clipId:first.clip_id};
    if(result.extracted)trackId=first.track_id;
  }
  updateArrangeSelection(null);
  accept(result.project);
  toast(result.extracted?'选段已收录':result.transition?'已应用交叉淡化':result.warp?'已保存对拍点':
    request.action.kind==='tempo'?'已保存片段的跟速设置':
    request.action.kind==='slip'?`已滑移 ${result.count} 个片段的内容`:`已编辑 ${result.count} 个片段`);
}

const clipSlipDialog=document.createElement('dialog');
clipSlipDialog.id='clipSlipDialog';
clipSlipDialog.className='arrange-dialog';
clipSlipDialog.setAttribute('aria-labelledby','clipSlipTitle');
clipSlipDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="clipSlipTitle">滑移片段内容</h2><button id="clipSlipClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>源音频偏移 · ms<input id="clipSlipAmount" type="number" step="any" min="-900000" max="900000"
      aria-describedby="clipSlipHint"></label>
    <small id="clipSlipHint">正值从源音频更晚处开始读。</small>
  </div>
  <output id="clipSlipStatus" role="status"></output>
  <div class="arrange-dialog-footer"><button id="previewClipSlip">预览</button><button class="primary" id="applyClipSlip" disabled>应用</button></div>`;
document.body.append(clipSlipDialog);
wireDialog(clipSlipDialog);
const clipSlipAudition=arrangementAuditionControls(clipSlipDialog);
let clipSlipTarget=null,clipSlipDraft=null;

function clipSlipFrames(){
  const amount=$('clipSlipAmount').valueAsNumber;
  return Math.sign(amount)*Math.round(Math.abs(amount)*48);
}
function showClipSlipState(message=''){
  const input=$('clipSlipAmount');
  input.disabled=clipBusy;
  $('previewClipSlip').disabled=clipBusy||!input.checkValidity()||!clipSlipFrames();
  $('applyClipSlip').disabled=clipBusy||!clipSlipDraft;
  $('clipSlipStatus').textContent=message;
}
async function openClipSlip(){
  if(clipBusy||clipSlipDialog.open||!project)return;
  await editChain.catch(()=>{});
  const target=groupRequest(null);
  if(!target.clips.length){toast('先选择要滑移的音频片段。',true);return}
  clipSlipTarget=target;
  clipSlipDraft=null;
  $('clipSlipAmount').value='';
  showClipSlipState(`已选 ${target.clips.length} 个片段`);
  clipSlipDialog.showModal();
  $('clipSlipAmount').focus();
}
$('clipSlip').onclick=$('groupSlip').onclick=openClipSlip;
$('clipSlipClose').onclick=()=>clipSlipDialog.close();
clipSlipDialog.addEventListener('close',()=>{clipSlipTarget=null;clipSlipDraft=null});
$('clipSlipAmount').oninput=()=>{
  clipSlipDraft=null;
  clipSlipAudition.set(null);
  showClipSlipState();
};
$('previewClipSlip').onclick=async()=>{
  const target=clipSlipTarget;
  if(!target||$('previewClipSlip').disabled)return;
  const frames=clipSlipFrames();
  const request={...target,action:{kind:'slip',source_delta_frames:frames}};
  clipSlipDraft=null;
  clipSlipAudition.set(null);
  clipBusy=true;
  showClipSlipState('正在预览…');
  let message='';
  try{
    const preview=await api('audio_clip_group',request);
    if(clipSlipTarget!==target)return;
    clipSlipDraft={request,id:preview.preview_id};
    bindClipAudition(clipSlipAudition,request,preview.preview_id);
    message=`${preview.count} 个片段 · ${frames>0?'+':''}${(frames/48).toFixed(3)} ms`;
  }catch(error){message=clipError(error.message)}
  finally{clipBusy=false;if(clipSlipTarget===target)showClipSlipState(message)}
};
$('applyClipSlip').onclick=async()=>{
  const target=clipSlipTarget,draft=clipSlipDraft;
  if(!draft||clipBusy)return;
  clipBusy=true;
  showClipSlipState('正在应用…');
  try{
    await applyClipGroup(draft.request,draft.id,clipSlipAudition.binding());
    if(clipSlipTarget===target)clipSlipDialog.close();
  }catch(error){
    if(clipSlipTarget===target){
      clipSlipDraft=null;
      clipSlipAudition.set(null);
      $('clipSlipStatus').textContent=clipError(error.message);
    }
  }finally{
    clipBusy=false;
    if(clipSlipTarget===target)showClipSlipState($('clipSlipStatus').textContent);
  }
};

const clipTempoDialog=document.createElement('dialog');
clipTempoDialog.id='clipTempoDialog';
clipTempoDialog.className='arrange-dialog';
clipTempoDialog.innerHTML=`
  <div class="dialog-heading"><h2>片段跟速</h2><button id="clipTempoClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="clip-tools">
    <label>模式<select id="clipTempoMode">
      <option value="span">跟速 · 使用当前拍数</option>
      <option value="beats">跟速 · 指定拍数</option>
      <option value="bpm">跟速 · 指定源 BPM</option>
      <option value="fixed">固定秒数</option>
    </select></label>
    <label id="clipTempoValueLabel" hidden><span id="clipTempoUnit"></span><input id="clipTempoValue" type="number" min="0" step="any"></label>
    <label id="clipTempoOverlapLabel" class="clip-check" hidden><input id="clipTempoOverlap" type="checkbox">允许这次重叠</label>
  </div>
  <output id="clipTempoStatus" role="status"></output>
  <div class="arrange-dialog-footer"><button id="previewClipTempo">预览</button><button class="primary" id="applyClipTempo" disabled>应用</button></div>`;
document.body.append(clipTempoDialog);
wireDialog(clipTempoDialog);
const clipTempoAudition=arrangementAuditionControls(clipTempoDialog);
$('clipTempoClose').onclick=()=>clipTempoDialog.close();
let clipTempoTarget=null,clipTempoDraft=null;

function showClipTempoState(message){
  const mode=$('clipTempoMode').value;
  $('clipTempoValueLabel').hidden=mode!=='beats'&&mode!=='bpm';
  $('clipTempoUnit').textContent=mode==='beats'?'目标拍数':'源 BPM';
  $('clipTempoStatus').textContent=message||'';
  $('clipTempoMode').disabled=$('clipTempoValue').disabled=$('clipTempoOverlap').disabled=$('previewClipTempo').disabled=clipBusy;
  $('applyClipTempo').disabled=clipBusy||!clipTempoDraft;
}
async function openClipTempo(){
  if(clipBusy)return;
  await editChain.catch(()=>{});
  const request=groupRequest({kind:'tempo'});
  if(!request.clips.length)return;
  clipTempoTarget=request;
  clipTempoDraft=null;
  clipTempoAudition.set(null);
  $('clipTempoMode').value='span';
  $('clipTempoValue').value='';
  $('clipTempoOverlap').checked=false;
  $('clipTempoOverlapLabel').hidden=true;
  showClipTempoState();
  clipTempoDialog.showModal();
  $('previewClipTempo').click();
}
$('clipTempo').onclick=$('groupTempo').onclick=openClipTempo;
clipTempoDialog.addEventListener('close',()=>{clipTempoTarget=null;clipTempoDraft=null});
for(const id of ['clipTempoMode','clipTempoValue','clipTempoOverlap'])$(id).oninput=()=>{
  clipTempoDraft=null;clipTempoAudition.set(null);showClipTempoState();
};
$('previewClipTempo').onclick=async()=>{
  const target=clipTempoTarget;
  if(!target||clipBusy)return;
  const mode=$('clipTempoMode').value;
  const action={kind:'tempo'};
  const value=$('clipTempoValue').valueAsNumber;
  if((mode==='beats'||mode==='bpm')&&!(value>0)){showClipTempoState('请输入正数');return}
  if(mode==='beats')action.beats=value;
  if(mode==='bpm')action.source_bpm=value;
  if(mode==='fixed')action.fixed=true;
  if($('clipTempoOverlap').checked)action.allow_overlap=true;
  const request={...target,action};
  clipBusy=true;
  clipTempoDraft=null;
  clipTempoAudition.set(null);
  showClipTempoState('正在计算时长…');
  let message;
  try{
    const preview=await api('audio_clip_group',request);
    if(clipTempoTarget!==target)return;
    clipTempoDraft={request,id:preview.preview_id};
    bindClipAudition(clipTempoAudition,request,preview.preview_id);
    const after=preview.after;
    if(after.length===1){
      const item=after[0],beats=item.beat_range[1]-item.beat_range[0];
      message=Number(beats.toFixed(3))+' 拍 · '+((item.end_frame-item.start_frame)/clipRate).toFixed(3)+' 秒';
    }else message=after.length+' 个片段 · '+(mode==='fixed'?'固定秒数':'跟随工程速度');
    const removed=mode==='fixed'?preview.before.reduce((count,item)=>count+(item.clip.warp_points?.length||0),0):0;
    if(removed)message+=' · 移除 '+removed+' 个对拍点';
  }catch(error){
    if(error.message.includes('CLIP_OVERLAP')){
      $('clipTempoOverlapLabel').hidden=false;
      message='新时长会与邻接片段重叠，请调整时长或允许重叠。';
    }else message=clipError(error.message);
  }
  finally{clipBusy=false;showClipTempoState(clipTempoTarget===target?message:null)}
};
$('applyClipTempo').onclick=async()=>{
  const target=clipTempoTarget,draft=clipTempoDraft;
  if(!target||!draft||clipBusy)return;
  clipBusy=true;
  showClipTempoState('正在保存…');
  let message;
  try{
    await applyClipGroup(draft.request,draft.id,clipTempoAudition.binding());
    if(clipTempoTarget===target)clipTempoDialog.close();
  }catch(error){clipTempoDraft=null;message=clipError(error.message)}
  finally{clipBusy=false;showClipTempoState(clipTempoTarget===target?message:null)}
};

function showCrossfadeState(message){
  $('crossfadeStatus').textContent=message||crossfadeTarget?.label||'';
  $('crossfadeLength').disabled=clipBusy||!!crossfadeTarget?.overlap;
  $('crossfadeCurve').disabled=$('previewCrossfade').disabled=clipBusy;
  $('applyCrossfade').disabled=clipBusy||!crossfadeDraft;
  $('crossfadePlot').hidden=!crossfadeDraft;
}

$('groupCrossfade').onclick=async()=>{
  if(clipBusy)return;
  await editChain.catch(()=>{});
  const items=clipGroupItems().sort((a,b)=>clipStartFrame(a.clip)-clipStartFrame(b.clip));
  if(items.length!==2||items[0].track.id!==items[1].track.id)return;
  const [left,right]=items.map(item=>item.clip);
  const overlap=clipEndFrame(left)-clipStartFrame(right);
  crossfadeTarget={...groupRequest({kind:'crossfade'}),overlap:Math.max(0,overlap),label:left.name+' / '+right.name};
  crossfadeDraft=null;
  crossfadeAudition.set(null);
  $('crossfadeLength').value=overlap>0?Number((overlap/48).toFixed(3)):'';
  $('crossfadeCurve').value='';
  showCrossfadeState();
  crossfadeDialog.showModal();
  $('previewCrossfade').click();
};
crossfadeDialog.addEventListener('close',()=>{crossfadeTarget=null;crossfadeDraft=null});
for(const id of ['crossfadeLength','crossfadeCurve'])$(id).oninput=()=>{
  crossfadeDraft=null;crossfadeAudition.set(null);showCrossfadeState();
};

$('previewCrossfade').onclick=async()=>{
  const target=crossfadeTarget;
  if(clipBusy||!target)return;
  const {project_id,expected_revision,clips}=target;
  const request={project_id,expected_revision,clips,action:{kind:'crossfade'}};
  const length=$('crossfadeLength'),curve=$('crossfadeCurve').value;
  if(length.value&&!target.overlap)request.action.length_frames=Math.round(length.valueAsNumber*48);
  if(curve)request.action.curve=curve;
  crossfadeDraft=null;
  crossfadeAudition.set(null);
  clipBusy=true;
  showCrossfadeState('正在计算淡化…');
  let message;
  try{
    if(length.validity.badInput)throw new Error('请输入有效时长');
    const result=await api('audio_clip_group',request);
    if(crossfadeTarget!==target)return;
    const transition=result.transition;
    const [start,end]=transition.frame_range;
    for(const [id,reverse] of [['crossfadeOut',true],['crossfadeIn',false]]){
      const points=Array.from({length:17},(_,i)=>{
        const progress=(reverse?16-i:i)/16;
        return (8+i*19)+','+(54-48*fadeGain(progress,transition.curve)).toFixed(2);
      });
      $(id).setAttribute('points',points.join(' '));
    }
    crossfadeDraft={request,id:result.preview_id};
    bindClipAudition(crossfadeAudition,request,result.preview_id);
    message=Number(((end-start)/48).toFixed(3))+' ms · '+(transition.curve==='linear'?'线性':'等功率');
  }catch(error){message=clipError(error.message)}
  finally{clipBusy=false;showCrossfadeState(crossfadeTarget===target?message:null)}
};
$('applyCrossfade').onclick=async()=>{
  const target=crossfadeTarget,draft=crossfadeDraft;
  if(clipBusy||!target||!draft)return;
  clipBusy=true;
  showCrossfadeState('正在保存…');
  let message;
  try{
    await applyClipGroup(draft.request,draft.id,crossfadeAudition.binding());
    if(crossfadeTarget===target)crossfadeDialog.close();
  }catch(error){crossfadeDraft=null;message=clipError(error.message)}
  finally{clipBusy=false;showCrossfadeState(crossfadeTarget===target?message:null)}
};

function duplicateClipGroup(){
  const clips=clipGroupItems();
  if(!clips.length)return;
  const start=Math.min(...clips.map(({clip})=>clip.start));
  const end=Math.max(...clips.map(({clip})=>clipEndBeat(clip)));
  const step=Number($('quantize').value);
  void quickGroupAction(groupRequest({kind:'copy',delta:Math.ceil(end/step)*step-start}));
}

function copyClipGroup(){
  const items=clipGroupItems();
  if(!items.length)return;
  audioGroupClipboard={...groupRequest(null),start:Math.min(...items.map(({clip})=>clip.start))};
  toast(`已复制 ${items.length} 个片段到剪贴板`);
}

function pasteClipGroup(){
  const draft=audioGroupClipboard;
  if(!draft||draft.project_id!==project?.id)return false;
  if(draft.expected_revision!==project.revision){toast('工程已更新，请重新复制片段',true);return true}
  const {start,...request}=draft;
  void quickGroupAction({...request,action:{kind:'copy',delta:cursorBeat-start}});
  return true;
}
$('groupCopy').onclick=duplicateClipGroup;
$('groupRemove').onclick=()=>void quickGroupAction(groupRequest({kind:'remove'}));

const originalArrangeDown=arrangeCanvas.onpointerdown;
arrangeCanvas.onpointerdown=event=>{
  if(!project||event.button!==0||clipBusy)return;
  const point=arrangePoint(event);
  const found=pickAudioClipAt(point);
  if(event.shiftKey){originalArrangeDown(event);return}
  if(found&&(event.ctrlKey||event.metaKey)){
    const current=clipGroupItems(),single=selectedAudioClip();
    if(!current.length&&single)current.push(single);
    const existing=current.findIndex(({track,clip})=>track.id===found.track.id&&clip.id===found.clip.id);
    if(existing<0)current.push(found);else current.splice(existing,1);
    updateArrangeSelection(null);setClipGroup(current);workspaceFocus='arrangement';arrangeViewport.focus({preventScroll:true});return;
  }
  if(!found){
    if(point.y>=arrangeHeader){
      const row=project.tracks[point.row];
      if(row&&point.x<arrangeLabelWidth){selectAudioClip(row,null);return}
      if(clipSelection||groupedClips.length)selectAudioClip(null,null);
    }
    originalArrangeDown(event);
    return;
  }
  const group=clipGroupItems();
  const draggingGroup=group.some(({track,clip})=>track.id===found.track.id&&clip.id===found.clip.id);
  if(!draggingGroup)selectAudioClip(found.track,found.clip);
  workspaceFocus='arrangement';
  arrangeViewport.focus({preventScroll:true});
  const length=(clipEndBeat(found.clip)-found.clip.start)*arrangeZoom;
  const first=(point.beat-found.clip.start)*arrangeZoom;
  const handle=length>=10?Math.min(7,length/4):0;
  const mode=draggingGroup?'group':first<handle?'left':length-first<handle?'right':'body';
  clipDrag={request:draggingGroup?groupRequest(null):clipRequest(),group,clip:found.clip,point,pointerId:event.pointerId,mode,copy:event.altKey,placement:null};
  try{arrangeCanvas.setPointerCapture(event.pointerId)}catch(error){if(error.name!=='NotFoundError')throw error}
};

const originalArrangeMove=arrangeCanvas.onpointermove;
arrangeCanvas.onpointermove=event=>{
  if(!clipDrag){originalArrangeMove(event);return}
  if(clipDrag.pointerId!==event.pointerId)return;
  const point=arrangePoint(event),{clip,mode}=clipDrag;
  if(Math.hypot(point.x-clipDrag.point.x,point.y-clipDrag.point.y)<5){clipDrag.placement=null;queueArrangementDraw();return}
  if(mode==='group'){
    const lanes=project.tracks.filter(isAudioTrack);
    const index=track=>lanes.findIndex(lane=>lane.id===track?.id);
    const rows=clipDrag.group.map(({track})=>index(track));
    const wanted=index(project.tracks[point.row]),initial=index(project.tracks[clipDrag.point.row]);
    const lane_offset=Math.max(-Math.min(...rows),Math.min(lanes.length-1-Math.max(...rows),wanted<0?0:wanted-initial));
    const first=Math.min(...clipDrag.group.map(item=>item.clip.start));
    const delta=Math.max(-first,snap(clip.start+point.beat-clipDrag.point.beat)-clip.start);
    const items=clipDrag.group.map(({track,clip:member})=>{
      const start=member.start+delta;
      return {start,end:clipEndBeat({...member,start}),row:project.tracks.indexOf(lanes[index(track)+lane_offset])};
    });
    clipDrag.placement={items,action:{kind:clipDrag.copy?'copy':'move',delta,lane_offset}};
  }else if(mode==='body'){
    const row=isAudioTrack(project.tracks[point.row])?point.row:clipDrag.point.row;
    const start=Math.max(0,snap(clip.start+point.beat-clipDrag.point.beat));
    const end=clipEndBeat({...clip,start});
    clipDrag.placement={row,start,end,action:{kind:clipDrag.copy?'copy':'move',start,to_track_id:project.tracks[row].id}};
  }else{
    const beforeProject=clip.beat_length?Math.ceil(mapClipWarp(clip,'beat','frame',-clip.start,true)-1e-8):-clipStartFrame(clip);
    const min=Math.max(-clip.offset,beforeProject);
    const max=(clipAssetFrames.get(clip.asset)??(clip.offset+clip.frames))-clip.offset;
    const relative=clipSourceAtBeat(clip,Math.max(0,snap(point.beat)),true);
    const first=mode==='left'?Math.max(min,Math.min(clip.frames-1,relative)):0;
    const end=mode==='right'?Math.max(1,Math.min(max,relative)):clip.frames;
    clipDrag.placement=first===0&&end===clip.frames?null:{
      row:clipDrag.point.row,start:clipBeatAtSource(clip,first,true),end:clipBeatAtSource(clip,end,true),
      action:{kind:'trim',first,end,allow_overlap:event.altKey}
    };
  }
  queueArrangementDraw();
};

const originalArrangeUp=arrangeCanvas.onpointerup;
arrangeCanvas.onpointerup=event=>{
  if(!clipDrag){originalArrangeUp(event);return}
  if(clipDrag.pointerId!==event.pointerId)return;
  const drag=clipDrag;clipDrag=null;queueArrangementDraw();
  if(!drag.placement){arrangementSeek(snap(drag.point.beat));return}
  const request={...drag.request,action:drag.placement.action};
  if(drag.mode==='group')void quickGroupAction(request);else void quickClipAction(request);
};
const originalArrangeCancel=arrangeCanvas.onpointercancel;
arrangeCanvas.onpointercancel=event=>{clipDrag=null;queueArrangementDraw();originalArrangeCancel(event)};
const originalArrangeDoubleClick=arrangeCanvas.ondblclick;
arrangeCanvas.ondblclick=event=>{
  const found=project&&pickAudioClipAt(arrangePoint(event));
  if(found){selectAudioClip(found.track,found.clip);openClipDialog('set');return}
  originalArrangeDoubleClick(event);
};

function refreshClipFields(){
  const kind=$('clipKind').value;
  for(const panel of $('clipFields').querySelectorAll('[data-clip-fields]')){
    panel.hidden=!panel.dataset.clipFields.split(' ').includes(kind);
  }
  clipPreview=null;$('clipApply').disabled=true;$('clipPreviewInfo').textContent='';
}
function openClipDialog(kind){
  const selected=selectedAudioClip();
  if(!selected||clipDialog.open)return;
  const {track,clip}=selected;
  clipDraft=clipRequest();
  $('clipLength').textContent=((clipEndFrame(clip)-clipStartFrame(clip))/clipRate).toFixed(3)+' 秒';
  $('clipKind').value=kind;
  $('clipName').value=clip.name;$('clipGain').value=clip.gain_db;
  for(const [side,label,key] of [['In','淡入','fade_in'],['Out','淡出','fade_out']]){
    const input=$('clipFade'+side);
    const beats=clip[key+'_beats'];
    input.dataset.unit=beats==null?'ms':'beats';
    input.value=beats??clip[key]/48;
    $('clipFade'+side+'Unit').textContent=label+' · '+(beats==null?'ms':'拍');
  }
  $('clipAt').value=cursorBeat;
  $('clipTargetBeat').value=kind==='copy'?clipEndBeat(clip):clip.start;
  $('clipTargetTrack').replaceChildren(...project.tracks.filter(isAudioTrack).map(t=>new Option(t.name,t.id)));
  $('clipTargetTrack').value=track.id;$('clipAllowOverlap').checked=false;
  $('clipFirst').value=0;$('clipEnd').value=clip.frames/clipRate;$('clipTrimOverlap').checked=false;
  refreshClipFields();
  clipDialog.showModal();
}

function readClipAction(){
  const kind=$('clipKind').value;
  const number=id=>{
    const value=$(id).valueAsNumber;
    if(!Number.isFinite(value))throw new Error('请输入正确的数值');
    return value;
  };
  switch(kind){
    case 'split':return {kind,at:number('clipAt')};
    case 'move':case 'copy':return {kind,start:number('clipTargetBeat'),to_track_id:$('clipTargetTrack').value,allow_overlap:$('clipAllowOverlap').checked};
    case 'trim':return {kind,first:Math.round(number('clipFirst')*clipRate),end:Math.round(number('clipEnd')*clipRate),
      allow_overlap:$('clipTrimOverlap').checked};
    case 'set':{
      const action={kind,name:$('clipName').value,gain_db:number('clipGain')};
      for(const [id,key] of [['clipFadeIn','fade_in'],['clipFadeOut','fade_out']]){
        if($(id).dataset.unit==='beats')action[key+'_beats']=number(id);
        else action[key]=Math.round(number(id)*48);
      }
      return action;
    }
    default:return {kind};
  }
}

$('clipKind').onchange=refreshClipFields;
clipDialog.addEventListener('input',()=>{clipPreview=null;$('clipApply').disabled=true;$('clipPreviewInfo').textContent=''});
$('clipPreview').onclick=async()=>{
  if(clipBusy)return;
  try{
    const request={...clipDraft,action:readClipAction()},key=JSON.stringify(request);
    clipBusy=true;$('clipPreview').disabled=true;
    const result=await api('audio_clip_edit',request);
    if(!clipDialog.open||JSON.stringify({...clipDraft,action:readClipAction()})!==key)return;
    clipPreview={request,id:result.preview_id};
    $('clipPreviewInfo').textContent=result.after.length
      ?result.after.map(item=>`${item.beat_range[0].toFixed(3)}–${item.beat_range[1].toFixed(3)} 拍`).join(' / ')+(result.fades_reset?' · 裁掉的片段边缘淡化已重置':'')
      :'选中的片段将从音轨移除';
    $('clipApply').disabled=false;
  }catch(error){$('clipPreviewInfo').textContent=clipError(error.message)}
  finally{clipBusy=false;$('clipPreview').disabled=false}
};
$('clipApply').onclick=async()=>{
  if(clipBusy||!clipPreview)return;
  clipBusy=true;$('clipApply').disabled=true;
  try{await applyClipEdit(clipPreview.request,clipPreview.id);clipDialog.close()}
  catch(error){$('clipPreviewInfo').textContent=clipError(error.message)}
  finally{clipBusy=false}
};

function makeSplitRequest(action){
  const single=selectedAudioClip();
  const group=single?[single]:clipGroupItems();
  const clips=group.filter(({clip})=>clip.start<action.at&&clipEndBeat(clip)>action.at)
    .map(({track,clip})=>({track_id:track.id,clip_id:clip.id}));
  if(!clips.length){
    if(group.length)toast('游标未穿过选中的片段');
    return null;
  }
  return {...groupRequest(action),clips};
}
function splitSelectedClip(){
  const action={kind:'split',at:cursorBeat};
  if(selectedAudioClip()){void quickClipAction({...clipRequest(),action});return}
  const request=makeSplitRequest(action);
  if(request)void quickGroupAction(request);
}
async function reviewSplit(){
  if(clipBusy||splitDialog.open)return;
  const request=makeSplitRequest({kind:'split',at:cursorBeat});
  if(!request)return;
  const review={request,id:null};
  splitReview=review;
  splitAudition.set(null);
  $('applySplit').disabled=true;
  $('splitStatus').textContent='正在检查切点…';
  splitDialog.showModal();
  try{
    await editChain.catch(()=>{});
    const preview=await api('audio_clip_group',request);
    if(splitReview!==review||!splitDialog.open)return;
    review.id=preview.preview_id;
    $('splitStatus').textContent=`${preview.count} 个片段 · 第 ${Number(request.action.at.toFixed(3))} 拍`;
    $('applySplit').disabled=false;
    bindClipAudition(splitAudition,request,review.id);
  }catch(error){
    if(splitReview===review&&splitDialog.open)$('splitStatus').textContent=clipError(error.message);
  }
}
$('applySplit').onclick=async()=>{
  const review=splitReview;
  if(clipBusy||!review?.id)return;
  clipBusy=true;
  $('applySplit').disabled=true;
  try{
    await applyClipGroup(review.request,review.id,splitAudition.binding());
    if(splitReview===review)splitDialog.close();
  }catch(error){
    if(splitReview===review)$('splitStatus').textContent=clipError(error.message);
  }finally{clipBusy=false}
};
$('clipSplit').onclick=reviewSplit;
$('groupSplit').onclick=reviewSplit;
$('clipCopy').onclick=()=>openClipDialog('copy');
$('clipSettings').onclick=()=>openClipDialog('set');
const refreshWithoutClips=refresh;
refresh=function(external=false){
  const live=clipGroupItems();
  if(live.length!==groupedClips.length){
    groupedClips=live.length>1?live.map(({track,clip})=>({projectId:project.id,track_id:track.id,clip_id:clip.id})):[];
    if(live.length===1)clipSelection={projectId:project.id,trackId:live[0].track.id,clipId:live[0].clip.id};
  }
  refreshWithoutClips(external);
  if(!selectedAudioClip())clipSelection=null;
  showClipTools();
};
const originalSwitchArrangement=switchArrangement;
switchArrangement=async function(show){await originalSwitchArrangement(show);showClipTools()};

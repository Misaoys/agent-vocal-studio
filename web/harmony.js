'use strict';

const harmonyButton=document.createElement('button');
harmonyButton.id='showHarmony';
harmonyButton.title='编辑和弦标记';
harmonyButton.innerHTML=icon('pitch')+'<span>和弦</span>';
$('showGainAutomation').after(harmonyButton);

const harmonyDialog=document.createElement('dialog');
harmonyDialog.className='arrange-dialog harmony-dialog';
harmonyDialog.setAttribute('aria-labelledby','harmonyTitle');
harmonyDialog.innerHTML=`<div class="dialog-heading"><h2 id="harmonyTitle">和弦标记</h2><button id="harmonyClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="harmony-toolbar"><output id="harmonyRange"></output><button id="harmonyAdd">${icon('plus')} 添加</button></div>
  <div id="harmonyRows"></div>
  <div class="arrange-dialog-footer"><button id="harmonyPrevious" aria-label="上一页">${icon('left')}</button>
  <button id="harmonyNext" aria-label="下一页">${icon('right')}</button><output id="harmonyStatus" role="status"></output>
  <button id="harmonyApply" class="primary">应用</button></div>`;
document.body.append(harmonyDialog);
wireDialog(harmonyDialog);

let harmonyDraft=null,harmonyPage=0,harmonyBusy=false;
const harmonyPageSize=14;
const harmonyRound=beat=>Number(beat.toFixed(3));

function paintHarmony(){
  if(!harmonyDraft)return;
  const {beat_range:range,points}=harmonyDraft;
  const pages=Math.max(1,Math.ceil(points.length/harmonyPageSize));
  harmonyPage=Math.min(harmonyPage,pages-1);
  const first=harmonyPage*harmonyPageSize;
  $('harmonyRange').textContent=range
    ?range.map(harmonyRound).join('–')+' 拍 · 保留选区外和弦'
    :'整曲 · '+points.length+' 点';
  $('harmonyPrevious').hidden=$('harmonyNext').hidden=pages===1;
  $('harmonyPrevious').disabled=harmonyPage===0;
  $('harmonyNext').disabled=harmonyPage===pages-1;
  $('harmonyRows').innerHTML=points.slice(first,first+harmonyPageSize).map((point,i)=>{
    const index=first+i,locked=range&&index===0;
    return `<div class="harmony-row" data-index="${index}">
      <label>位置 · 拍<input type="number" data-field="beat" min="0" max="50000" step="any" value="${point.beat??''}" ${locked?'disabled':''}></label>
      <label>和弦<input type="text" data-field="symbol" maxlength="38" value="${esc(point.symbol)}" placeholder="Dm9/F" spellcheck="false"></label>
      <button aria-label="删除标记" data-harmony-remove ${locked?'disabled':''}>${icon('close')}</button></div>`;
  }).join('');
  $('harmonyApply').disabled=harmonyBusy||JSON.stringify(points)===harmonyDraft.original;
}

async function openHarmony(at){
  if(!project||harmonyDialog.open||harmonyBusy)return;
  const binding={project_id:project.id,expected_revision:project.revision};
  const chosen=at===undefined&&workspaceFocus==='arrangement'?arrangeSelection:null;
  const range=chosen?[chosen.start,chosen.end]:undefined;
  harmonyBusy=true;
  try{
    const result=await api('harmony_query',{...binding,beat_range:range});
    if(project?.id!==binding.project_id||project.revision!==binding.expected_revision)throw new Error('工程已更新，请重新打开和弦');
    const points=result.markers.map(point=>({...point}));
    if(range&&points[0]?.beat!==range[0]){
      const prior=result.chords.find(chord=>chord.beat_range[0]===range[0]);
      points.unshift({beat:range[0],symbol:prior?.symbol??'N.C.'});
    }
    const index=at===undefined?0:points.findLastIndex(point=>point.beat<=at);
    harmonyPage=Math.floor(Math.max(0,index)/harmonyPageSize);
    harmonyDraft={...binding,beat_range:range,points,original:JSON.stringify(points)};
    $('harmonyStatus').textContent='';
    harmonyDialog.showModal();
    paintHarmony();
    if(index>=0)$('harmonyRows').querySelector('[data-index="'+index+'"] input[data-field=symbol]')?.focus();
  }catch(error){toast(error.message,true)}
  finally{harmonyBusy=false;paintHarmony()}
}

harmonyButton.onclick=()=>openHarmony();
$('harmonyClose').onclick=()=>harmonyDialog.close();
$('harmonyAdd').onclick=()=>{
  if(!harmonyDraft||harmonyBusy||harmonyDraft.points.length>=1500)return;
  const {points,beat_range:range}=harmonyDraft;
  const last=points.at(-1)?.beat??0;
  let beat=points.length?Math.max(last+1,cursorBeat):cursorBeat;
  if(range&&beat>=range[1])beat=(last+range[1])/2;
  if(range&&!(beat>last&&beat<range[1]))return;
  const pretty=harmonyRound(Math.min(50000,beat));
  points.push({beat:range&&(pretty<=last||pretty>=range[1])?beat:pretty,symbol:points.at(-1)?.symbol??'C'});
  harmonyPage=Math.floor((points.length-1)/harmonyPageSize);
  paintHarmony();
  $('harmonyRows').lastElementChild?.querySelector('input[data-field=symbol]')?.select();
};
for(const [id,delta] of [['harmonyPrevious',-1],['harmonyNext',1]])$(id).onclick=()=>{harmonyPage+=delta;paintHarmony()};
$('harmonyRows').oninput=event=>{
  const element=event.target,row=element.closest('[data-index]');
  if(!row||!harmonyDraft||harmonyBusy)return;
  const point=harmonyDraft.points[Number(row.dataset.index)];
  point[element.dataset.field]=element.dataset.field==='beat'?element.valueAsNumber:element.value;
  $('harmonyApply').disabled=JSON.stringify(harmonyDraft.points)===harmonyDraft.original;
};
$('harmonyRows').onclick=event=>{
  const remove=event.target.closest('[data-harmony-remove]');
  if(!remove||!harmonyDraft||harmonyBusy)return;
  harmonyDraft.points.splice(Number(remove.closest('[data-index]').dataset.index),1);
  paintHarmony();
};
$('harmonyApply').onclick=async()=>{
  const draft=harmonyDraft;
  if(!draft||harmonyBusy)return;
  const points=draft.points.map(point=>({beat:point.beat,symbol:point.symbol.trim()})).sort((a,b)=>a.beat-b.beat);
  if(points.some(point=>!Number.isFinite(point.beat)||point.beat<0||point.beat>50000||!point.symbol)){
    $('harmonyStatus').textContent='请输入有效拍位和和弦名';return;
  }
  harmonyBusy=true;paintHarmony();
  try{
    const operation=draft.beat_range?{op:'set_harmony_range',beat_range:draft.beat_range,markers:points}:
      {op:'set_project',values:{harmony:points}};
    await edit([operation],draft);
    harmonyDialog.close();
  }catch(error){$('harmonyStatus').textContent=error.message}
  finally{harmonyBusy=false;paintHarmony()}
};
harmonyDialog.addEventListener('close',()=>{harmonyDraft=null});

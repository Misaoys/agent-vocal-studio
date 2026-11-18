'use strict';

const gainButton=document.createElement('button');
gainButton.id='showGainAutomation';
gainButton.title='编辑音轨的音量曲线';
gainButton.innerHTML=icon('sliders')+'<span>音量曲线</span>';
$('showMixer').after(gainButton);

const gainDialog=document.createElement('dialog');
gainDialog.className='arrange-dialog gain-dialog';
gainDialog.setAttribute('aria-labelledby','gainAutomationTitle');
gainDialog.innerHTML=`<div class="dialog-heading"><h2 id="gainAutomationTitle">音量曲线</h2><button id="gainClose" aria-label="关闭">${icon('close')}</button></div>
  <div class="gain-toolbar"><select id="gainTrack" aria-label="自动化音轨"></select><output id="gainBeatRange"></output></div>
  <svg id="gainPlot" viewBox="0 0 560 160" aria-label="横轴为拍，纵轴为相对推子的音量增益"></svg>
  <div class="gain-controls"><label>位置 · 拍<input id="gainPointBeat" type="number" min="0" max="50000" step="any"></label>
  <label>增益 · dB<input id="gainPointDb" type="number" min="-60" max="12" step="any"></label>
  <button id="gainAdd">${icon('plus')} 添加</button><button id="gainDelete" aria-label="移除选中的点">${icon('close')}</button></div>
  <div class="arrange-dialog-footer"><output id="gainStatus" role="status"></output><button id="gainApply" class="primary">应用</button></div>`;
document.body.append(gainDialog);
wireDialog(gainDialog);

const gainSvg=$('gainPlot');
let gainDraft=null,gainDrag=null,gainLoad=0,gainSaving=false;
const gainClamp=(value,min,max)=>Math.max(min,Math.min(max,value));
const gainRound=value=>Number(value.toFixed(3));

function gainSvgPart(name,attrs,text){
  const node=document.createElementNS('http://www.w3.org/2000/svg',name);
  for(const [key,value] of Object.entries(attrs))node.setAttribute(key,value);
  if(text!=null)node.textContent=text;
  gainSvg.append(node);
  return node;
}

function drawGainCurve(){
  const d=gainDraft;
  if(!d)return;
  gainSvg.replaceChildren();
  const [start,end]=d.domain;
  const x=beat=>40+510*(beat-start)/(end-start),y=db=>12+124*(12-db)/72;
  for(const db of [12,0,-24,-60]){
    gainSvgPart('line',{x1:40,x2:550,y1:y(db),y2:y(db),class:db===0?'gain-zero':''});
    gainSvgPart('text',{x:33,y:y(db)+4,'text-anchor':'end'},db);
  }
  gainSvgPart('text',{x:40,y:153},gainRound(start));
  gainSvgPart('text',{x:550,y:153,'text-anchor':'end'},gainRound(end));
  const all=d.bounds?[{beat:start,db:d.bounds[0]},...d.points,{beat:end,db:d.bounds[1]}]:d.points;
  const clock=clockFor(project),points=[];
  let upper=0;
  const beats=[...Array.from({length:101},(_,i)=>start+(end-start)*i/100),...all.map(p=>p.beat)].sort((a,b)=>a-b);
  for(const beat of beats){
    while(upper<all.length&&all[upper].beat<=beat)upper++;
    const low=all[upper-1],high=all[upper];
    let db=(low||high)?.db??0;
    if(low&&high){
      const a=clock.seconds(low.beat),b=clock.seconds(high.beat);
      const weight=b>a?(clock.seconds(beat)-a)/(b-a):0;
      db=20*Math.log10(10**(low.db/20)*(1-weight)+10**(high.db/20)*weight);
    }
    points.push(x(beat)+','+y(db));
  }
  gainSvgPart('polyline',{points:points.join(' ')});
  for(const point of d.points){
    const marker=gainSvgPart('circle',{cx:x(point.beat),cy:y(point.db),r:5,tabindex:0,
      class:point===d.selected?'selected':'','aria-label':gainRound(point.beat)+' 拍，'+gainRound(point.db)+' dB'});
    marker.onpointerdown=event=>{
      if(event.button!==0||gainSaving)return;
      d.selected=point;
      gainDrag={point,pointer:event.pointerId,original:{...point}};
      gainSvg.setPointerCapture(event.pointerId);
      drawGainCurve();
    };
    marker.onkeydown=event=>{
      if(event.key==='Enter'||event.key===' '){
        event.preventDefault();d.selected=point;drawGainCurve();$('gainPointDb').focus();
      }
    };
  }
  $('gainPointBeat').disabled=$('gainPointDb').disabled=$('gainDelete').disabled=!d.selected||gainSaving;
  $('gainPointBeat').value=d.selected?gainRound(d.selected.beat):'';
  $('gainPointDb').value=d.selected?gainRound(d.selected.db):'';
  $('gainApply').disabled=gainSaving||JSON.stringify(d.points)===d.original;
}

function moveGainPoint(point,beat,db){
  const d=gainDraft,[from,to]=d.domain;
  if(!Number.isFinite(beat)||!Number.isFinite(db))return false;
  if(beat<from||beat>to||d.bounds&&(beat<=from||beat>=to)||db<-60||db>12||
    d.points.some(other=>other!==point&&other.beat===beat))return false;
  point.beat=beat;point.db=db;
  d.points.sort((a,b)=>a.beat-b.beat);
  return true;
}

function gainPointer(event){
  const point=new DOMPoint(event.clientX,event.clientY).matrixTransform(gainSvg.getScreenCTM().inverse());
  const [a,b]=gainDraft.domain;
  return [gainRound(a+(b-a)*gainClamp((point.x-40)/510,0,1)),Number((12-72*gainClamp((point.y-12)/124,0,1)).toFixed(1))];
}

function addGainPoint(beat,db){
  if(!gainDraft||gainSaving||gainDraft.points.length>=2000)return;
  const point={};
  if(!moveGainPoint(point,beat,db))return;
  gainDraft.points.push(point);
  gainDraft.points.sort((a,b)=>a.beat-b.beat);
  gainDraft.selected=point;
  drawGainCurve();
}

gainSvg.ondblclick=event=>{if(gainDraft&&event.target.tagName!=='circle')addGainPoint(...gainPointer(event))};
gainSvg.onpointermove=event=>{
  if(gainDrag?.pointer===event.pointerId&&moveGainPoint(gainDrag.point,...gainPointer(event)))drawGainCurve();
};
gainSvg.onlostpointercapture=()=>{gainDrag=null};
gainSvg.onpointercancel=()=>{
  if(!gainDrag)return;
  moveGainPoint(gainDrag.point,gainDrag.original.beat,gainDrag.original.db);
  gainDrag=null;drawGainCurve();
};
$('gainAdd').onclick=()=>{
  if(!gainDraft)return;
  const [start,end]=gainDraft.domain,anchors=[start,...gainDraft.points.map(p=>p.beat),end];
  if(!gainDraft.bounds&&!gainDraft.points.length){addGainPoint(gainClamp(cursorBeat,start,end),0);return}
  let widest=0;
  for(let i=1;i<anchors.length-1;i++)if(anchors[i+1]-anchors[i]>anchors[widest+1]-anchors[widest])widest=i;
  addGainPoint((anchors[widest]+anchors[widest+1])/2,0);
};
$('gainDelete').onclick=()=>{
  if(!gainDraft?.selected||gainSaving)return;
  gainDraft.points=gainDraft.points.filter(point=>point!==gainDraft.selected);
  gainDraft.selected=null;drawGainCurve();
};
for(const field of ['gainPointBeat','gainPointDb'])$(field).onchange=event=>{
  const selected=gainDraft?.selected;
  if(!selected||gainSaving)return;
  const beat=field==='gainPointBeat'?event.target.valueAsNumber:selected.beat;
  const db=field==='gainPointDb'?event.target.valueAsNumber:selected.db;
  $('gainStatus').textContent=moveGainPoint(selected,beat,db)?'':'点必须在范围内，拍位不可重复';
  drawGainCurve();
};

async function loadGainCurve(track,range,binding){
  gainDraft=null;gainDrag=null;
  gainSvg.replaceChildren();
  $('gainApply').disabled=$('gainPointBeat').disabled=$('gainPointDb').disabled=$('gainDelete').disabled=true;
  const request=++gainLoad;
  $('gainStatus').textContent='正在读取…';
  try{
    const value=await api('gain_automation_query',{...binding,track_id:track,beat_range:range});
    if(request!==gainLoad||!gainDialog.open)return;
    if(project?.id!==binding.project_id||project.revision!==binding.expected_revision)throw new Error('工程已更新，请重新打开');
    const domain=range||[0,Math.max(arrangementModel().limit,value.points.at(-1)?.beat||0)];
    const points=range?value.points.filter(p=>p.beat>range[0]&&p.beat<range[1]):value.points;
    gainDraft={track,range,binding,domain,bounds:value.boundary_db,points,original:JSON.stringify(points),selected:null};
    $('gainBeatRange').textContent=range?gainRound(range[0])+'–'+gainRound(range[1])+' 拍 · 边界锁定':'整轨';
    $('gainStatus').textContent='';
    drawGainCurve();
  }catch(error){if(request===gainLoad)$('gainStatus').textContent=error.message}
}

gainButton.onclick=()=>{
  if(!project||gainDialog.open)return;
  const range=arrangeSelection;
  const tracks=range?project.tracks.filter(track=>range.ids.includes(track.id)):project.tracks;
  if(!tracks.length)return;
  const binding={project_id:project.id,expected_revision:project.revision};
  $('gainTrack').replaceChildren(...tracks.map(track=>new Option(track.name,track.id)));
  if(tracks.some(track=>track.id===trackId))$('gainTrack').value=trackId;
  $('gainTrack').onchange=()=>loadGainCurve($('gainTrack').value,range?[range.start,range.end]:undefined,binding);
  gainDialog.showModal();
  $('gainTrack').onchange();
};
$('gainClose').onclick=()=>gainDialog.close();
gainDialog.addEventListener('close',()=>{gainDraft=null;gainDrag=null;gainLoad++});
$('gainApply').onclick=async()=>{
  const d=gainDraft;
  if(!d||gainSaving)return;
  const operation=d.range?{op:'set_gain_range',track_id:d.track,beat_range:d.range,points:d.points}:
    {op:'update_track',track_id:d.track,values:{gain_automation:d.points}};
  gainSaving=true;
  $('gainApply').disabled=$('gainTrack').disabled=true;
  try{
    await edit([operation],d.binding);
    if(gainDraft===d)gainDialog.close();
  }catch(error){
    if(gainDraft===d)$('gainStatus').textContent=error.message;
  }finally{
    gainSaving=false;$('gainTrack').disabled=false;
    if(gainDraft===d)drawGainCurve();
  }
};

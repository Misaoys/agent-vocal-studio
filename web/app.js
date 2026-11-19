'use strict';
const $=id=>document.getElementById(id),token=document.querySelector('meta[name="studio-token"]').content;
let project=null,trackId=null,selected=null,projects=[],singers=[],zoom=76,rowH=22,keyW=62,rulerH=40,minPitch=48,maxPitch=84,mode='pitch',drag=null,rendered=null,rendering=false,editChain=Promise.resolve(),toastTimer;
const lastProjectPreference='avs-last-project';
let projectLoadRequest=0;
const projectPageSize=40;
let projectListRequest=0,projectPage=0,projectSearchTimer;
let playingArchivedMix=false;
const audio=window.desktopNative?new DesktopAudio():$('audio'),roll=$('roll'),ctx=roll.getContext('2d'),expr=$('expression'),ex=expr.getContext('2d');
function toast(msg,error=false){$('toast').textContent=msg;$('toast').className='visible'+(error?' error':'');clearTimeout(toastTimer);toastTimer=setTimeout(()=>$('toast').className='',6000)}
async function api(name,args={}){if(window.desktopNative)return window.desktopNative.invoke('call',{name,arguments:args});const r=await fetch('/rpc',{method:'POST',headers:{'Content-Type':'application/json','X-Agent-Vocal-Token':token},body:JSON.stringify({name,arguments:args,source:"user"})});const v=await r.json();if(!v.ok)throw new Error(v.error);return v.data}
function tr(){return project?.tracks.find(t=>t.id===trackId)||project?.tracks[0]}
function note(){return tr()?.notes.find(n=>n.id===selected)}
function esc(s){return String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]))}
function accept(p,external=false){
  const switched=p.id!==project?.id;
  const revisionChanged=switched||p.revision!==project?.revision;
  const voicesChanged=switched?p.tracks.some(track=>track.openutau_voice):
    p.tracks.some(track=>track.openutau_voice!==project.tracks.find(item=>item.id===track.id)?.openutau_voice);
  const timelineChanged=switched||project&&(project.bpm!==p.bpm||JSON.stringify(project.tempo_map||[])!==JSON.stringify(p.tempo_map||[]));
  const staleTruePeak=revisionChanged&&!audio.liveMix&&!audio.monitorStarting&&
    project?.master_limiter?.peak_mode==='true_peak';
  if(project&&(timelineChanged||voicesChanged||staleTruePeak||playingArchivedMix&&revisionChanged)){
    playingArchivedMix=false;
    if(window.desktopNative)audio.removeAttribute('src');
    else{audio.pause();audio.currentTime=0;audio.removeAttribute('src')}
    rendered=null;
    if(timelineChanged&&typeof cursorBeat==='number')cursorBeat=0;
  }
  project=p;
  if(voicesChanged)$('renderEngine').value='project';
  if(switched){
    projectLoadRequest++;
    resetAgentPresence();
  }else if(revisionChanged){
    invalidateAgentHighlight(p.revision);
  }
  if(window.desktopNative&&revisionChanged)audio.setProject(p).catch(error=>toast(error.message,true));
  if(!p.tracks.some(t=>t.id===trackId)){trackId=p.tracks[0].id;selected=null}
  if(!note())selected=null;
  if(rendered&&(rendered.project_id!==p.id||rendered.revision!==p.revision))rendered=null;
  refresh(external);
  if(switched){
    try{localStorage.setItem(lastProjectPreference,p.id)}catch{}
  }
}
async function load(id){
  const request=++projectLoadRequest;
  let opened;
  try{opened=await api('project_read',{project_id:id})}
  catch(error){
    if(request===projectLoadRequest)throw error;
    return;
  }
  if(request!==projectLoadRequest)return;
  if(opened.id!==project?.id||opened.revision>=project.revision)accept(opened);
  $('rollViewport').scrollTop=Math.max(0,(maxPitch-72)*rowH);
  if(!$('projects').inert)paintProjectList();
}
function projectCatalogScope(){return $('projectTests').getAttribute('aria-pressed')==='true'?'technical':'workspace'}
function projectCatalogQuery(){return $('projectSearch').value.trim()}
function paintProjectList(){
  const list=$('projects');
  const pinCurrent=project&&projectPage===0&&!projectCatalogQuery()&&projectCatalogScope()==='workspace'&&
    !projects.some(row=>row.id===project.id);
  const records=pinCurrent?[{id:project.id,title:project.title},...projects]:projects;
  const existing=new Map([...list.children].map(element=>[element.dataset.project,element]));
  let anchor=list.firstElementChild;
  for(const record of records){
    let row=existing.get(record.id);
    if(row){
      existing.delete(record.id);
    }else{
      row=document.createElement('button');
      row.className='project-item';
      row.dataset.project=record.id;
      const folder=document.createElement('b');
      folder.className='folder';
      folder.textContent='▱';
      row.append(folder,document.createElement('span'));
      row.onclick=()=>load(record.id).catch(error=>toast(error.message,true));
    }
    const title=record.id===project?.id?project.title:record.title;
    if(row.title!==title){
      row.title=title;
      row.lastElementChild.textContent=title;
    }
    row.classList.toggle('active',record.id===project?.id);
    if(row!==anchor)list.insertBefore(row,anchor);
    anchor=row.nextElementSibling;
  }
  for(const row of existing.values())row.remove();
}
function projectListStatus(message){
  const status=$('projectListStatus');
  status.hidden=!message;
  status.textContent=message;
}
async function refreshList({force=false}={}){
  const list=$('projects'),drawer=list.closest('dialog');
  if(!force&&drawer&&!drawer.open)return;
  clearTimeout(projectSearchTimer);
  const request=++projectListRequest;
  const scope=projectCatalogScope(),query=projectCatalogQuery();
  try{
    const incoming=await api('project_list',{scope,query,limit:projectPageSize+1,offset:projectPage*projectPageSize});
    if(request!==projectListRequest)return;
    if(!incoming.length&&projectPage>0){
      projectPage=0;
      return refreshList({force});
    }
    projects=incoming.slice(0,projectPageSize);
    const more=incoming.length>projectPageSize;
    paintProjectList();
    $('projectPages').hidden=!projectPage&&!more;
    $('projectPrev').disabled=projectPage===0;
    $('projectNext').disabled=!more;
    $('projectPage').textContent=String(projectPage+1);
    projectListStatus(list.children.length?'':query?'没有匹配的工程':'暂无工程');
    return projects;
  }catch(error){
    if(request!==projectListRequest)return;
    projectListStatus(error.message);
    throw error;
  }finally{
    if(request===projectListRequest){
      list.inert=false;
      list.removeAttribute('aria-busy');
    }
  }
}
function refreshProjectSearch(delay){
  projectListRequest++;
  const list=$('projects');
  list.inert=true;
  list.setAttribute('aria-busy','true');
  list.scrollTop=0;
  $('projectPrev').disabled=true;
  $('projectNext').disabled=true;
  projectListStatus('');
  clearTimeout(projectSearchTimer);
  projectSearchTimer=setTimeout(()=>void refreshList().catch(()=>{}),delay);
}
const queueProjectSearch=()=>{projectPage=0;refreshProjectSearch(160)};
$('projectSearch').oninput=event=>{if(!event.isComposing)queueProjectSearch()};
$('projectSearch').oncompositionend=queueProjectSearch;
$('projectTests').onclick=()=>{
  const button=$('projectTests'),active=projectCatalogScope()==='technical';
  button.setAttribute('aria-pressed',String(!active));
  button.title=active?'切换到技术测试工程':'切换到作品工程';
  projectPage=0;
  refreshProjectSearch(0);
};
$('projectPrev').onclick=()=>{projectPage--;refreshProjectSearch(0)};
$('projectNext').onclick=()=>{projectPage++;refreshProjectSearch(0)};
function edit(operations,guard={}){const target=guard.project_id||project?.id;editChain=editChain.catch(()=>{}).then(async()=>{if(!target)return;$('saveState').textContent='正在保存…';try{if(!project||project.id!==target)throw new Error('工程已切换，排队修改未保存');const saved=await api('project_edit',{project_id:target,expected_revision:guard.expected_revision??project.revision,operations:typeof operations==='function'?operations():operations});if(project?.id===target)accept(saved);$('saveState').textContent=project?.id===target?'已自动保存':'上个工程已保存';await refreshList();return saved}catch(e){$('saveState').textContent='修改未保存';toast(e.message,true);if(e.message.includes('REVISION_CONFLICT')&&project?.id===target)await load(target);throw e}});return editChain}

function updateNote(values){if(!note())return;const nid=selected,tid=trackId;return edit([{op:'update_note',track_id:tid,note_id:nid,values}])}
function updateTrack(values){if(!tr())return;return edit([{op:'update_track',track_id:trackId,values}])}
function beatX(beat){return keyW+beat*zoom}function pitchY(p){return rulerH+(maxPitch-p)*rowH}
function beats(){return Math.max(16,Math.ceil((Math.max(0,...(tr()?.notes||[]).map(n=>n.start+n.duration))+4)/4)*4)}
function canvasSize(canvas,w,h){const dpr=window.devicePixelRatio||1;if(canvas.style.width!==w+'px'||canvas.style.height!==h+'px'||canvas.width!==Math.round(w*dpr)){canvas.style.width=w+'px';canvas.style.height=h+'px';canvas.width=Math.round(w*dpr);canvas.height=Math.round(h*dpr)}const c=canvas.getContext('2d');c.setTransform(dpr,0,0,dpr,0,0);return c}
function refresh(external=false){if(!project)return;const t=tr();document.querySelector('.track-swatch').style.background=trackColor(t);$('projectTitle').value=project.title;$('trackTitle').textContent=t.name;$('noteCount').textContent=t.notes.length+' 音符';$('bpm').value=project.bpm;const meter=project.meter.join('/');if(![...$('meter').options].some(o=>o.value===meter))$('meter').add(new Option(meter,meter));$('meter').value=meter;$('revision').textContent='REV '+String(project.revision).padStart(3,'0');$('emptyHint').hidden=t.notes.length>0;
  $('tracks').innerHTML=project.tracks.map(t=>`<div class="track-card ${t.id===trackId?'active':''}" data-track="${t.id}" role="button" tabindex="0" aria-label="选择音轨 ${esc(t.name)}" aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown Ctrl+Shift+D"><div class="track-top"><span class="track-dot" style="background:${trackColor(t)}"></span><span class="track-name">${esc(t.name)}</span><button class="${t.mute?'on':''}" data-mute="${t.id}" aria-pressed="${t.mute}" aria-label="静音 ${esc(t.name)}">M</button><button class="${t.solo?'on':''}" data-solo="${t.id}" aria-pressed="${t.solo}" aria-label="独奏 ${esc(t.name)}">S</button></div><small>${isAudioTrack(t)?trackClips(t).length+' 片段':t.instrument?'乐器':'歌声'} · ${t.gain_db>0?'+':''}${t.gain_db} dB</small><button class="track-duplicate" data-duplicate-track="${t.id}" aria-label="复制音轨 ${esc(t.name)}" title="复制音轨 · Ctrl+Shift+D">${icon('copy')}</button><span class="track-grip" data-track-order aria-hidden="true" title="拖动重排音轨 · Alt+↑ / ↓">${icon('grip')}</span></div>`).join('');
  for(const card of $('tracks').querySelectorAll('.track-card')){const select=()=>{const track=project.tracks.find(t=>t.id===card.dataset.track);chooseGroup([]);updateArrangeSelection(null);selectAudioClip(track,null);arrangeViewport.focus({preventScroll:true})};card.onclick=event=>{if(!event.target.closest('[data-track-order],[data-duplicate-track]'))select()};card.onkeydown=e=>{if(e.target===card&&(e.key==='Enter'||e.key===' ')){e.preventDefault();select()}}}
  for(const b of $('tracks').querySelectorAll('[data-mute],[data-solo]'))b.onclick=e=>{e.stopPropagation();const tid=b.dataset.mute||b.dataset.solo,field=b.dataset.mute?'mute':'solo';edit([{op:'update_track',track_id:tid,values:{[field]:!project.tracks.find(t=>t.id===tid)[field]}}]).catch(()=>{})};
  const ns=t.notes.map(n=>n.pitch);minPitch=Math.min(48,...ns.map(n=>Math.max(0,n-4)));maxPitch=Math.max(84,...ns.map(n=>Math.min(127,n+4)));
  $('trackName').value=t.name;$('gain').value=t.gain_db;$('gainValue').textContent=t.gain_db+' dB';$('pan').value=t.pan;$('panValue').textContent=t.pan===0?'居中':(t.pan<0?'左 ':'右 ')+Math.round(Math.abs(t.pan)*100);paintTrackSinger(t);
  inspect();draw();if(external){$('saveState').textContent='Agent 更新已同步';toast('已同步 Agent 对工程的修改')}
}
function inspect(){
  const n=note(),v=n?.vibrato;
  $('noteInspector').classList.toggle('disabled',!n);
  $('selectedIndicator').textContent=n?'已选择':'未选择';
  for(const k of ['lyric','pitch','velocity','start','duration'])$(k).value=n?.[k]??'';
  $('vibratoDepth').value=v?.depth??0;
  $('depthValue').textContent=(v?.depth??0)+' cent';
  $('vibratoRate').value=v?.rate??5.5;
  $('vibratoStart').max=(v?.end??1)*100;
  $('vibratoStart').value=(v?.start??.6)*100;
  inspectConsonant(n);
}
function curveCents(note,beat){
  const points=note.pitch_points;
  if(!points.length)return 0;
  const index=upperTimeline(points,0,beat),[at,cents]=points[index],next=points[index+1];
  if(!next)return cents;
  const fraction=Math.max(0,Math.min(1,(beat-at)/(next[0]-at)));
  return cents+fraction*(next[1]-cents);
}
let consonantTarget='', consonantReading=null;
function inspectConsonant(n){
  const t=tr(),visible=!!t&&!t.instrument&&!isAudioTrack(t);
  const tie=$('vowelContinuation');
  tie.closest('label').hidden=!visible;
  tie.checked=!!n?.vowel_continuation;
  const previous=n&&t?.notes.find(candidate=>candidate.id!==n.id&&Math.abs(n.start-candidate.start-candidate.duration)<=1e-7);
  tie.disabled=!visible||!n||(!tie.checked&&!previous);
  $('lyric').disabled=tie.checked;
  $('consonantControls').hidden=!visible||tie.checked;
  if(!visible||tie.checked)n=null;
  const key=n?JSON.stringify([project.id,project.revision,t.id,n.id]):'';
  if(key!==consonantTarget){
    consonantTarget=key;
    consonantReading=null;
    $('consonantResult').textContent='';
  }
  $('readConsonant').disabled=!n;
  $('consonantMs').disabled=!n||(!consonantReading?.consonant&&n.consonant_ms==null);
  $('consonantMs').value=n?.consonant_ms??'';
  $('vowelOffset').disabled=!n||(consonantReading?.vowel_onset_seconds==null&&n.vowel_offset_ms==null);
  $('vowelOffset').value=n?.vowel_offset_ms??'';
}
$('vowelContinuation').onchange=async()=>{
  const n=note();
  if(!n)return;
  try{
    await edit([{op:'update_note',track_id:trackId,note_id:n.id,values:{
      vowel_continuation:$('vowelContinuation').checked
    }}]);
  }catch{inspect()}
};
$('readConsonant').onclick=async()=>{
  const target=consonantTarget,n=note();
  if(!n||!target)return;
  $('readConsonant').disabled=true;
  $('consonantResult').textContent='正在分析…';
  try{
    const query=await api('pronunciation_query',{project_id:project.id,expected_revision:project.revision,track_id:trackId,note_ids:[n.id]});
    if(target!==consonantTarget)return;
    consonantReading=query.notes[0];
    const timing=consonantReading;
    const ms=value=>Number(value.toFixed(1));
    const lines=[];
    if(timing.consonant){
      const audible=timing.audible_lead_ms<timing.effective_lead_ms?' · 起点后可听 '+ms(timing.audible_lead_ms)+' ms':'';
      lines.push(timing.consonant+' · 自动 '+ms(timing.default_lead_ms)+' · 当前 '+ms(timing.effective_lead_ms)+' · 上限 '+ms(timing.max_lead_ms)+' ms'+audible);
      $('consonantMs').disabled=false;
    }else lines.push('这个音符没有可调的前置子音');
    if(timing.vowel_onset_seconds!=null){
      const [min,max]=timing.vowel_offset_range_ms.map(ms);
      lines.push('元音偏移 '+ms(timing.effective_vowel_offset_ms)+' ms · 范围 '+min+' 至 '+max+' ms'+(timing.vowel_offset_limited?' · 已限位':''));
      $('vowelOffset').disabled=false;
    }
    $('consonantResult').replaceChildren(lines.join('。'));
  }catch(error){
    if(target===consonantTarget)$('consonantResult').textContent=error.message;
  }finally{
    if(target===consonantTarget)$('readConsonant').disabled=false;
  }
};
function savePronunciation(event,field,available,range){
  const input=event.target,n=note();
  if(!n||input.disabled)return;
  const value=input.value===''?null:input.valueAsNumber;
  if(!input.validity.valid||(value!==null&&!available)){
    input.value=n[field]??'';
    toast(range,true);
    return;
  }
  if(value===(n[field]??null))return;
  const tid=trackId;
  const guard={project_id:project.id,expected_revision:project.revision};
  edit([{op:'update_note',track_id:tid,note_id:n.id,values:{[field]:value}}],guard)
    .then(()=>{
      if(project?.id===guard.project_id&&trackId===tid&&selected===n.id)$('readConsonant').click();
    }).catch(()=>inspect());
}
$('consonantMs').onchange=event=>savePronunciation(event,'consonant_ms',!!consonantReading?.consonant,'请输入 1–300 ms，留空恢复自动');
$('vowelOffset').onchange=event=>savePronunciation(event,'vowel_offset_ms',consonantReading?.vowel_onset_seconds!=null,'请输入 -120 至 120 ms，留空恢复原始落拍');
function noteCents(n,b,seconds=beatSeconds){
  const cents=curveCents(n,b),v=n.vibrato;
  const start=n.duration*v.start,end=n.duration*(v.end??1);
  if(!v.depth||end<=start||b<=start||b>=end&&(v.end!=null||v.fade_in_ms!=null||v.fade_out_ms!=null))return cents;
  const time=seconds(n.start+b),elapsed=time-seconds(n.start+start);
  const attack=v.fade_in_ms??80,release=v.fade_out_ms??0;
  const rise=attack>0?elapsed*1000/attack:1;
  const fall=release>0?(seconds(n.start+end)-time)*1000/release:1;
  return cents+v.depth*Math.min(1,rise,fall)*Math.sin(2*Math.PI*v.rate*elapsed);
}
function draw(){if(!project)return;const t=tr(),w=Math.max(beatX(beats()),$('rollViewport').clientWidth),h=(maxPitch-minPitch+1)*rowH+rulerH;canvasSize(roll,w,h);ctx.fillStyle='#0b0b0c';ctx.fillRect(0,0,w,h);const names=['C','C♯','D','D♯','E','F','F♯','G','G♯','A','A♯','B'];
  for(let p=minPitch;p<=maxPitch;p++){const y=pitchY(p),black=[1,3,6,8,10].includes(p%12);ctx.fillStyle=black?'#101012':'#0b0b0c';ctx.fillRect(keyW,y,w-keyW,rowH);ctx.strokeStyle='#212125';ctx.beginPath();ctx.moveTo(0,y+rowH);ctx.lineTo(w,y+rowH);ctx.stroke();ctx.fillStyle=black?'#242427':'#2b2b31';ctx.fillRect(0,y,keyW-1,rowH-1);ctx.fillStyle=p%12===0?'#cfcfd2':'#a3a3a7';ctx.font='9px Studio Inter, Studio Han, sans-serif';ctx.fillText(names[p%12]+(Math.floor(p/12)-1),p%12===0?13:19,y+14)}
  const q=Number($('quantize').value);for(let b=0;b<=beats();b+=q){const beatline=Math.abs(b-Math.round(b))<.0001;ctx.strokeStyle=beatline?'#2b2b31':'#242427';ctx.beginPath();ctx.moveTo(beatX(b),rulerH);ctx.lineTo(beatX(b),h);ctx.stroke()}
  for(const b of clockFor().bars(beats())){ctx.strokeStyle='#4d4d52';ctx.beginPath();ctx.moveTo(beatX(b.beat),rulerH);ctx.lineTo(beatX(b.beat),h);ctx.stroke()}timelineRuler(ctx,w);
  paintAgentPreview(ctx,audio,beatX,rulerH,h-rulerH);
  const scoreColor=trackColor(t),normalNote=scoreNotePalette(scoreColor,false),selectedNote=scoreNotePalette(scoreColor,true);
  for(const n of t.notes){const x=beatX(n.start),y=pitchY(n.pitch)+2,nw=Math.max(4,n.duration*zoom-2),isSel=n.id===selected,palette=isSel?selectedNote:normalNote;ctx.fillStyle=palette.fill;ctx.beginPath();ctx.roundRect(x+1,y,nw,rowH-4,3);ctx.fill();if(isSel){ctx.strokeStyle='#f0f0f1';ctx.lineWidth=1;ctx.stroke()}ctx.save();ctx.beginPath();ctx.rect(x+4,y,nw-6,rowH-4);ctx.clip();ctx.fillStyle=palette.ink;ctx.font='11px Studio Han, sans-serif';if(nw>12)ctx.fillText(typeof displayNote==='function'?displayNote(n,t):n.lyric,x+7,y+13);ctx.restore();
    if(n.pitch_points.length||n.vibrato.depth){ctx.strokeStyle='#f7fbff';ctx.lineWidth=1.2;ctx.beginPath();for(let i=0;i<=Math.max(2,nw/3);i++){const f=i/Math.max(2,Math.floor(nw/3));const xx=x+n.duration*zoom*Math.min(1,f),yy=pitchY(n.pitch)+rowH/2-noteCents(n,n.duration*Math.min(1,f))/100*rowH;if(i===0)ctx.moveTo(xx,yy);else ctx.lineTo(xx,yy)}ctx.stroke();ctx.lineWidth=1}
  }
  if(!window.desktopNative&&audio.currentTime>0){const x=beatX(secondsBeat(audio.currentTime));ctx.strokeStyle='#cfcfd2';ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,h);ctx.stroke()}drawExpression(w)
}
function pitchExpressionRange(n){
  if(!n)return 200;
  if(curveDrag?.nid===n.id&&curveDrag.range)return curveDrag.range;
  const peak=n.pitch_points.reduce((max,point)=>Math.max(max,Math.abs(point[1])),0);
  return Math.max(200,Math.ceil((peak+n.vibrato.depth)/100)*100);
}
function drawExpression(w){const range=pitchExpressionRange(note());canvasSize(expr,w,105);ex.fillStyle='#0b0b0c';ex.fillRect(0,0,w,105);ex.fillStyle='#151517';ex.fillRect(0,0,keyW,105);ex.font='9px Studio Inter, Studio Han, sans-serif';ex.fillStyle='#9f9fa6';ex.fillText(mode==='pitch'?'+'+range:'127',10,15);ex.fillText(mode==='pitch'?'−'+range:'0',10,95);for(let b=0;b<=beats();b++){ex.strokeStyle='#26262a';ex.beginPath();ex.moveTo(beatX(b),0);ex.lineTo(beatX(b),105);ex.stroke()}ex.strokeStyle='#39393e';ex.setLineDash([3,4]);ex.beginPath();ex.moveTo(keyW,52);ex.lineTo(w,52);ex.stroke();ex.setLineDash([]);
  if(mode==='velocity'){for(const n of tr().notes){const x=beatX(n.start),y=100-n.velocity/127*94;ex.fillStyle=n.id===selected?trackColor(tr()):trackColor(tr())+'92';ex.fillRect(x,y,Math.max(2,n.duration*zoom-3),100-y)}}
  else{const n=note();if(!n)return;ex.fillStyle='#b0b0b016';ex.fillRect(beatX(n.start),0,n.duration*zoom,105);ex.strokeStyle='#a5a5ad';ex.lineWidth=1.6;ex.beginPath();for(let i=0;i<=n.duration*zoom/2;i++){const b=i*2/zoom,x=beatX(n.start+b),y=52-noteCents(n,b)/range*46;if(i===0)ex.moveTo(x,y);else ex.lineTo(x,y)}ex.stroke();ex.lineWidth=1;for(const pt of n.pitch_points){ex.fillStyle='#cccccf';ex.beginPath();ex.arc(beatX(n.start+pt[0]),52-pt[1]/range*46,3,0,Math.PI*2);ex.fill()}}
}
function point(e,canvas=roll){const r=canvas.getBoundingClientRect();return{x:e.clientX-r.left,y:e.clientY-r.top}}
function hit(x,y){const b=(x-keyW)/zoom,p=maxPitch-Math.floor((y-rulerH)/rowH);return tr()?.notes.find(n=>n.pitch===p&&b>=n.start&&b<=n.start+n.duration)}
function snap(v){const q=Number($('quantize').value);return Math.round(v/q)*q}
roll.addEventListener('pointerdown',e=>{if(!project||e.button!==0)return;const pt=point(e),n=hit(pt.x,pt.y);if(!n){selected=null;inspect();draw();return}selected=n.id;drag={id:n.id,track:trackId,x:pt.x,y:pt.y,original:structuredClone(n),resize:pt.x>beatX(n.start+n.duration)-10,moved:false};roll.setPointerCapture(e.pointerId);inspect();draw()});
roll.addEventListener('pointermove',e=>{const pt=point(e);if(!drag){const n=hit(pt.x,pt.y);roll.style.cursor=n?(pt.x>beatX(n.start+n.duration)-10?'ew-resize':'grab'):'crosshair';return}const n=note(),o=drag.original;drag.moved||=Math.abs(pt.x-drag.x)+Math.abs(pt.y-drag.y)>3;if(!drag.moved)return;if(drag.resize){const d=Math.max(Number($('quantize').value),snap(o.duration+(pt.x-drag.x)/zoom));n.duration=d;n.pitch_points=o.pitch_points.filter(p=>p[0]<=d);if(o.pitch_blend)n.pitch_blend=o.pitch_blend.filter(p=>p[0]<=d)}else{n.start=Math.max(0,snap(o.start+(pt.x-drag.x)/zoom));n.pitch=Math.max(0,Math.min(127,o.pitch-Math.round((pt.y-drag.y)/rowH)))}inspect();draw()});
async function finishDrag(){if(!drag)return;const d=drag,n=note();drag=null;if(!d.moved||!n)return;const values={start:n.start,duration:n.duration,pitch:n.pitch,pitch_points:n.pitch_points,pitch_blend:n.pitch_blend};Object.assign(n,d.original);try{await edit([{op:'update_note',track_id:d.track,note_id:d.id,values}])}catch{await load(project.id)}}
roll.addEventListener('pointerup',finishDrag);roll.addEventListener('pointercancel',()=>{if(drag){Object.assign(note(),drag.original);drag=null;refresh()}});
roll.addEventListener('dblclick',e=>{const pt=point(e);if(!project||pt.x<keyW||pt.y<rulerH)return;const n=hit(pt.x,pt.y);if(n){selected=n.id;inspect();(tr()?.instrument?$('pitch'):$('lyric')).focus();(tr()?.instrument?$('pitch'):$('lyric')).select();return}const start=Math.max(0,snap((pt.x-keyW)/zoom)),pitch=maxPitch-Math.floor((pt.y-rulerH)/rowH),duration=Number($('quantize').value)*2;edit([{op:'add_notes',track_id:trackId,notes:[{start,duration,pitch,lyric:tr()?.instrument?'':'あ'}]}]).then(()=>{selected=tr().notes.find(n=>n.start===start&&n.pitch===pitch)?.id;inspect();draw()}).catch(()=>{})});
let curveDrag=null;function setExpression(e){const pt=point(e,expr);if(pt.x<keyW)return;if(mode==='velocity'){const b=(pt.x-keyW)/zoom,n=tr()?.notes.find(n=>b>=n.start&&b<=n.start+n.duration);if(!n)return;selected=n.id;curveDrag={nid:n.id,original:structuredClone(n)};n.velocity=Math.max(1,Math.min(127,Math.round((100-pt.y)/94*127)));inspect();draw()}
  else{const n=note();if(!n)return;const beat=Math.max(0,Math.min(n.duration,(pt.x-keyW)/zoom-n.start));if(!curveDrag)curveDrag={nid:n.id,track:trackId,original:structuredClone(n),range:pitchExpressionRange(n)};const range=curveDrag.range,limit=Math.min(range,2400);const b=Math.min(n.duration,Math.round(beat*480)/480),c=Math.max(-limit,Math.min(limit,Math.round((52-pt.y)/46*range)));n.pitch_points=n.pitch_points.filter(p=>Math.abs(p[0]-b)>.025);n.pitch_points.push([b,c]);n.pitch_points.sort((a,b)=>a[0]-b[0]);draw()}}
expr.onpointerdown=e=>{if(e.button!==0)return;curveDrag=null;setExpression(e);expr.setPointerCapture(e.pointerId)};expr.onpointermove=e=>{if(curveDrag&&e.buttons===1){if(mode==='velocity'){const n=note();if(n){n.velocity=Math.max(1,Math.min(127,Math.round((100-point(e,expr).y)/94*127)));inspect();draw()}}else setExpression(e)}};expr.onpointerup=()=>{if(!curveDrag)return;const n=note(),draft=curveDrag;curveDrag=null;if(!n||n.id!==draft.nid)return;const values=draft.range?{pitch_points:n.pitch_points}:{velocity:n.velocity};Object.assign(n,draft.original);if(draft.range)edit([{op:'update_note',track_id:draft.track,note_id:draft.nid,values}]).catch(()=>{});else updateNote(values)?.catch(()=>{})};expr.onpointercancel=()=>{if(curveDrag){Object.assign(note(),curveDrag.original);curveDrag=null;refresh()}};
$('rollViewport').onscroll=()=>{$('expressionViewport').scrollLeft=$('rollViewport').scrollLeft};
for(const k of ['lyric','pitch','velocity','start','duration'])$(k).onchange=()=>{const value=k==='lyric'?$(k).value:Number($(k).value);updateNote({[k]:value})?.catch(()=>{})};
const vibratoFields={vibratoDepth:'depth',vibratoRate:'rate',vibratoStart:'start'};
$('vibratoDepth').oninput=()=>{$('depthValue').textContent=$('vibratoDepth').value+' cent'};
for(const [id,key] of Object.entries(vibratoFields))$(id).onchange=()=>{
  const n=note(),input=$(id);
  if(!n||!input.validity.valid||!Number.isFinite(input.valueAsNumber)){inspect();return}
  const value=input.valueAsNumber/(key==='start'?100:1);
  updateNote({vibrato:{...n.vibrato,[key]:value}})?.catch(()=>{});
};
$('clearPitch').onclick=()=>updateNote({pitch_points:[],pitch_blend:[]})?.catch(()=>{});$('deleteNote').onclick=()=>{if(selected)edit([{op:'delete_notes',track_id:trackId,note_ids:[selected]}]).catch(()=>{})};
$('trackName').onchange=()=>updateTrack({name:$('trackName').value})?.catch(()=>{});$('gain').oninput=()=>{$('gainValue').textContent=$('gain').value+' dB'};$('gain').onchange=()=>updateTrack({gain_db:Number($('gain').value)})?.catch(()=>{});$('pan').oninput=()=>{$('panValue').textContent=$('pan').value};$('pan').onchange=()=>updateTrack({pan:Number($('pan').value)})?.catch(()=>{});
$('singer').onchange=()=>{
  const value=$('singer').value,track=tr(),scope=project?.id;
  if(!track)return;
  const openutau=value.startsWith('openutau:');
  const values=openutau?{openutau_voice:value.slice(9)}:{openutau_voice:null,singer_id:value===''?null:Number(value)};
  updateTrack(values)?.then(()=>{
    if(project?.id===scope&&(openutau||track.openutau_voice))$('renderEngine').value='project';
  }).catch(()=>paintTrackSinger(tr()));
};
$('projectTitle').onchange=()=>edit([{op:'set_project',values:{title:$('projectTitle').value}}]).catch(()=>{});$('bpm').onchange=()=>edit([{op:'set_project',values:{bpm:Number($('bpm').value)}}]).catch(()=>{});$('meter').onchange=()=>edit([{op:'set_project',values:{meter:$('meter').value.split('/').map(Number)}}]).catch(()=>{});
$('newProject').onclick=async()=>{try{await editChain.catch(()=>{});const p=await api('project_create',{title:'未命名声乐工程',bpm:120});accept(p);await refreshList()}catch(e){toast(e.message,true)}};
$('addTrack').onclick=()=>edit([{op:'add_track',values:{name:'和声 '+project.tracks.length}}]).then(()=>{trackId=project.tracks.at(-1).id;selected=null;refresh()}).catch(()=>{});
async function history(redo){if(!project)return;try{await editChain.catch(()=>{});accept(await api(redo?'project_redo':'project_undo',{project_id:project.id,expected_revision:project.revision}))}catch(e){toast(e.message,true)}}$('undo').onclick=()=>history(false);$('redo').onclick=()=>history(true);
$('quantize').onchange=draw;$('zoomIn').onclick=()=>{zoom=Math.min(190,zoom*1.25);draw()};$('zoomOut').onclick=()=>{zoom=Math.max(32,zoom/1.25);draw()};$('pitchTab').onclick=()=>{mode='pitch';$('pitchTab').classList.add('active');$('velocityTab').classList.remove('active');draw()};$('velocityTab').onclick=()=>{mode='velocity';$('velocityTab').classList.add('active');$('pitchTab').classList.remove('active');draw()};
let lyricsTarget=null,lyricsPlan=null,lyricsBusy=false;
function refreshLyrics(status){
  $('lyricsCount').textContent=status||lyricsTarget?.label||'';
  $('lyricsRead').disabled=lyricsBusy||!$('lyricsSource').value.trim();
  $('lyricsPreview').disabled=lyricsBusy||!$('lyricsText').value.trim();
  $('applyLyrics').disabled=lyricsBusy||!lyricsPlan?.hasChanges;
  $('lyricsSource').readOnly=$('lyricsText').readOnly=$('lyricsMode').disabled=lyricsBusy;
  if(!lyricsPlan){$('lyricsAlignment').hidden=true;$('lyricsAlignment').replaceChildren()}
}
$('batchLyrics').onclick=async()=>{
  try{
    await editChain;
    const ids=new Set(currentStudioSelection().note_ids);
    const tracks=(project?.tracks||[]).filter(track=>track.notes.some(note=>ids.has(note.id)));
    if(tracks.length>1)throw new Error('请从同一歌声轨选中要落字的乐句');
    const track=tracks[0]||tr();
    if(!track||track.instrument||isAudioTrack(track))throw new Error('请选择歌声轨');
    const phrase=track.notes.filter(note=>!ids.size||ids.has(note.id)).sort((a,b)=>a.start-b.start);
    const notes=phrase.filter(note=>!note.vowel_continuation);
    if(!phrase.length)throw new Error('请选择需要落字的歌声音符');
    const phraseIds=phrase.map(note=>note.id);
    const source=(track.lyric_lines||[]).find(line=>line.note_ids.length===phraseIds.length&&
      line.note_ids.every((id,index)=>id===phraseIds[index]));
    const altered=source&&source.reading.some((reading,index)=>reading!==(phrase[index].vowel_continuation?'+':phrase[index].lyric));
    lyricsTarget={project_id:project.id,expected_revision:project.revision,note_ids:notes.map(note=>note.id),
      fit_note_ids:phraseIds,phrase,source,label:track.name+' · '+phrase.length+' 个音符'+(altered?' · 请核对原词':'')};
    lyricsPlan=null;
    $('lyricsSource').value=source?.text||'';
    $('lyricsText').value=notes.map(note=>note.lyric).join(' ');
    $('lyricsMode').value=notes.length?'kana':'fit';
    refreshLyrics();
    $('lyricsDialog').showModal();
  }catch(error){toast(error.message,true)}
};
for(const id of ['lyricsSource','lyricsText','lyricsMode'])$(id).oninput=()=>{lyricsPlan=null;refreshLyrics()};
$('lyricsDialog').addEventListener('close',()=>{lyricsTarget=null;lyricsPlan=null});
$('lyricsRead').onclick=async()=>{
  const target=lyricsTarget;
  if(lyricsBusy||!target)return;
  lyricsBusy=true;
  lyricsPlan=null;
  refreshLyrics('正在提取本地读音…');
  let message;
  try{
    const result=await api('lyrics_reading',{mode:'japanese',text:$('lyricsSource').value.trim()});
    if(lyricsTarget!==target)return;
    $('lyricsText').value=result.reading;
    if($('lyricsMode').value==='manual')$('lyricsMode').value='kana';
    message=target.label+' · '+result.moras.length+' 个读音';
  }catch(error){message=error.message}
  finally{lyricsBusy=false;refreshLyrics(lyricsTarget===target?message:null)}
};
$('lyricsPreview').onclick=async()=>{
  const target=lyricsTarget;
  if(lyricsBusy||!target)return;
  lyricsBusy=true;
  lyricsPlan=null;
  refreshLyrics('正在对齐读音…');
  let message;
  try{
    if(project?.id!==target.project_id||project.revision!==target.expected_revision)throw new Error('工程已更新，请重新选择乐句');
    const reading=$('lyricsText').value.trim(),mode=$('lyricsMode').value;
    const written=$('lyricsSource').value.trim();
    const capturesSource=!!(written||target.source);
    const note_ids=mode==='fit'||capturesSource?target.fit_note_ids:target.note_ids;
    if(!note_ids.length)throw new Error('请选中读音起点');
    const selection=await api('selection_resolve',{project_id:target.project_id,note_ids});
    let transform={kind:mode==='fit'?'fit_lyrics':'align_lyrics',reading};
    if(mode==='manual'){
      let lyrics=reading.split(/\s+/);
      if(capturesSource){
        if(lyrics.length!==target.note_ids.length)throw new Error('请为每个起音输入一个读音');
        let onset=0;
        lyrics=target.phrase.map(note=>note.vowel_continuation?note.lyric:lyrics[onset++]);
      }
      transform={kind:'set_lyrics',lyrics};
    }
    if(capturesSource)transform.written_text=written||null;
    const args={selection_id:selection.selection_id,expected_revision:target.expected_revision,transform};
    const preview=await api('selection_edit',args);
    if(lyricsTarget!==target)return;
    lyricsPlan={args,hasChanges:preview.change_count>0||!!preview.lyric_line};
    const groups=preview.lyric_fit?.groups;
    const alignment=groups?groups.map(group=>({
      start:group.beat_range[0],middle:group.note_ids.length+' 音',
      reading:group.reading+(group.note_ids.length>1?'〜':''),seconds:group.seconds,timingReset:group.timing_reset
    })):preview.changes.slice().sort((a,b)=>a.before.start-b.before.start).map(change=>({
      start:change.before.start,middle:change.before.lyric,reading:change.after.lyric
    }));
    const rows=alignment.map(item=>{
      const row=document.createElement('li');
      const beat=document.createElement('span'),before=document.createElement('span'),after=document.createElement('strong');
      beat.textContent=Number((item.start+1).toFixed(3))+' 拍';
      before.textContent=item.middle;
      after.textContent=item.reading;
      if(item.seconds!==undefined)row.title=item.seconds.toFixed(3)+' 秒';
      if(item.timingReset)row.title+=' · 发音微调恢复自动';
      row.append(beat,before,after);
      return row;
    });
    $('lyricsAlignment').replaceChildren(...rows);
    $('lyricsAlignment').hidden=false;
    message=target.label+' · '+alignment.length+' 个读音已对齐';
    if(preview.lyric_line?.action==='save')message+=' · 将保存原词';
    if(preview.lyric_line?.action==='remove')message+=' · 将移除此句原词';
    const timingResets=alignment.filter(item=>item.timingReset).length;
    if(timingResets)message+=' · '+timingResets+' 处发音微调将恢复自动';
    if(!lyricsPlan.hasChanges)message+=' · 已是建议分配';
  }catch(error){message=error.message}
  finally{lyricsBusy=false;refreshLyrics(lyricsTarget===target?message:null)}
};
$('applyLyrics').onclick=async()=>{
  const target=lyricsTarget,plan=lyricsPlan;
  if(lyricsBusy||!target||!plan?.hasChanges)return;
  lyricsBusy=true;
  refreshLyrics('正在保存落字…');
  let message;
  try{
    await editChain;
    const saved=await api('selection_edit',{...plan.args,dry_run:false});
    if(project?.id===saved.project.id)accept(saved.project);
    if(lyricsTarget===target)$('lyricsDialog').close();
    await refreshList();
  }catch(error){lyricsPlan=null;message=error.message}
  finally{lyricsBusy=false;refreshLyrics(lyricsTarget===target?message:null)}
};
async function importFile(file){if(!file)return;try{if(file.size>32e6)throw new Error('曲谱文件超过 32 MB');const data=new Uint8Array(await file.arrayBuffer());let binary='';for(let i=0;i<data.length;i+=32768)binary+=String.fromCharCode(...data.subarray(i,i+32768));const v=await api('project_import',{filename:file.name,data_base64:btoa(binary)});accept(v.project);await refreshList();toast(v.warnings?.length?v.warnings.join(' '):'曲谱已导入为新工程')}catch(e){toast(e.message,true)}}$('fileInput').onchange=e=>{importFile(e.target.files[0]);e.target.value=''};$('importBtn').onclick=$('emptyImport').onclick=()=>$('fileInput').click();document.ondragover=e=>e.preventDefault();document.ondrop=e=>{e.preventDefault();importFile(e.dataTransfer.files[0])};
async function download(url){if(window.desktopNative){const saved=await studioSave(url);if(saved)toast('已保存导出');return}const a=document.createElement('a');a.href=url;a.download=decodeURIComponent(url.split('/').at(-1));a.click()}
async function exportFile(format){if(!project)return;try{await editChain.catch(()=>{});const v=await api('project_export',{project_id:project.id,format});await download(v.url);if(v.warning)toast(v.warning)}catch(e){toast(e.message,true)}}$('exportProject').onclick=()=>exportFile('avproj');$('exportMidi').onclick=()=>exportFile('mid');
let render, previewEpoch=0;
$('play').onclick=()=>{if(!audio.paused){audio.pause();return}render(true)};$('exportWav').onclick=()=>render(false,true);$('stop').onclick=()=>{previewEpoch++;if(window.desktopNative)audio.stop();else{audio.pause();audio.currentTime=0}draw()};audio.onplay=()=>{$('play').textContent='Ⅱ';if(!window.desktopNative)animate()};audio.onpause=audio.onended=()=>{$('play').textContent='▶';draw()};function animate(){draw();if(!audio.paused)requestAnimationFrame(animate)}
let openutauProfiles=[];
function paintTrackSinger(track){
  if(!track)return;
  const select=$('singer');
  select.querySelector('[data-missing-singer]')?.remove();
  const assigned=track.openutau_voice;
  const value=assigned?'openutau:'+assigned:String(track.singer_id??'');
  if(![...select.options].some(option=>option.value===value)){
    const unavailable=new Option(assigned?'缺少声库 · '+assigned:'缺少歌手 · '+value,value);
    unavailable.disabled=true;
    unavailable.dataset.missingSinger='';
    select.add(unavailable);
  }
  select.value=value;
}
function paintSingerCatalog(){
  const select=$('singer');
  select.replaceChildren(new Option('默认歌手',''));
  for(const [label,options] of [
    ['VOICEVOX',singers.flatMap(bank=>bank.styles.filter(style=>style.type==='frame_decode')
      .map(style=>new Option(bank.name+' · '+style.name,style.id)))],
    ['OpenUtau',openutauProfiles.map(profile=>new Option(profile.name,'openutau:'+profile.id))]
  ]){
    if(!options.length)continue;
    const group=document.createElement('optgroup');
    group.label=label;group.append(...options);select.append(group);
  }
  paintTrackSinger(tr());
}
async function engineCheck(){
  try{
    const v=await api('studio_status'),ready=v.engine.connected;
    const profiles=v.openutau?.profiles||[];
    const engine=$('renderEngine'),option=engine.querySelector('[value=openutau]');
    if(v.openutau?.configured&&!option)engine.add(new Option('OpenUtau · '+v.openutau.name,'openutau'));
    else if(!v.openutau?.configured&&option){if(option.selected)engine.value='guide';option.remove()}
    $('engineDot').classList.toggle('ready',ready||profiles.length>0);
    $('engineLabel').innerHTML=ready?'本地歌声引擎已连接<small>VOICEVOX · 无需订阅</small>':
      profiles.length?'本地歌声引擎<small>OpenUtau · '+esc(profiles.length===1?profiles[0].name:profiles.length+' 个声库')+'</small>':
      '歌声引擎未连接<small>仍可编辑与预览旋律</small>';
    const changed=JSON.stringify(profiles)!==JSON.stringify(openutauProfiles);
    openutauProfiles=profiles;
    const needsSingers=ready&&!singers.length;
    if(needsSingers)singers=await api('singer_list');
    if(changed||needsSingers)paintSingerCatalog();
  }catch(e){$('engineLabel').textContent='连接中断';$('saveState').textContent='连接中断'}
}
window.addEventListener('resize',draw);window.addEventListener('beforeunload',e=>{if($('saveState').textContent==='正在保存…'){e.preventDefault();e.returnValue=''}});
async function restoreWorkspace(){
  let opening=projectLoadRequest;
  const listing=refreshList({force:true}).then(items=>({items}),error=>({error}));
  let remembered;
  try{remembered=localStorage.getItem(lastProjectPreference)}catch{}
  const requested=window.studioStartupProject;
  for(const id of new Set([requested,remembered])){
    if(project||projectLoadRequest!==opening)break;
    if(!id)continue;
    opening++;
    try{
      await load(id);
      if(project?.id===id&&requested&&requested!==id)
        toast('找不到指定的工程，已打开工作区里的可用工程',true);
    }catch{}
  }
  if(project||projectLoadRequest!==opening)return;
  const {items,error}=await listing;
  if(project||projectLoadRequest!==opening)return;
  if(error)throw error;
  const [latest]=items&&!projectCatalogQuery()&&projectPage===0&&projectCatalogScope()==='workspace'?items:
    await api('project_list',{scope:'workspace',limit:1});
  if(project||projectLoadRequest!==opening)return;
  if(!latest){
    const created=await api('project_create',{title:'我的第一个声乐工程',bpm:120});
    if(project||projectLoadRequest!==opening)return;
    accept(created);
    await refreshList({force:true});
    return;
  }
  const initial=latest.id;
  await load(initial);
  if(requested&&initial!==requested)toast('找不到指定的工程，已打开工作区里的可用工程',true);
}
async function refreshVisibleRevision(){
  const current=project;
  if(!current||drag||curveDrag||trackOrderDrag||['INPUT','TEXTAREA','SELECT'].includes(document.activeElement.tagName))return;
  const latest=await api('project_read',{project_id:current.id});
  if(latest.id!==project?.id||latest.revision<=project.revision)return;
  accept(latest,true);
  await refreshList();
}
(async()=>{
  try{
    await restoreWorkspace();
    await engineCheck();
    setInterval(()=>void refreshVisibleRevision().catch(()=>{}),2500);
    setInterval(engineCheck,15000);
  }catch(error){toast(error.message,true)}
})();

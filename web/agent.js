'use strict';
let eventSequence=0,journalReady=false,followAgent=true,focusGeneration=0;
let agentRender=null,pendingAgentTarget=null,lastAgentOrder=0,showingAgentIntent=false;
const activityBar=document.createElement('section');activityBar.className='agent-activity';activityBar.dataset.agentPhase='idle';activityBar.setAttribute('aria-label','Agent 工作状态');activityBar.innerHTML='<div class="agent-copy" role="status" aria-atomic="true"><svg class="agent-signal" viewBox="0 0 22 14" aria-hidden="true"><circle cx="3" cy="7" r="1.5"/><circle cx="11" cy="7" r="1.5"/><circle cx="19" cy="7" r="1.5"/></svg><span id="agentPhase">Agent 待命</span><strong id="agentAction"></strong><span id="agentOutcome" hidden></span></div><button id="followAgent" class="on" aria-pressed="true">◎ 跟随 Agent</button><button id="showDesign">作品设计</button><button id="reviewSong">曲谱复核</button><button id="showKnowledge">创作知识 ↗</button>';
document.querySelector('.editor-shell').before(activityBar);
const agentCursor=document.createElement('div');agentCursor.id='agentCursor';agentCursor.setAttribute('aria-hidden','true');agentCursor.innerHTML='<svg width="25" height="25" viewBox="0 0 25 25"><polygon points="4,4 21,12 10,21" fill="#ededed20" stroke="#eeeeef" stroke-width="1.6" stroke-linejoin="round"/></svg>';document.body.append(agentCursor);
$('followAgent').onclick=()=>{followAgent=!followAgent;$('followAgent').classList.toggle('on',followAgent);$('followAgent').setAttribute('aria-pressed',String(followAgent));if(!followAgent)agentCursor.classList.remove('visible')};
function showCursor(x,y,element){agentCursor.style.transform=`translate(${Math.max(3,Math.min(innerWidth-27,x-4))}px,${Math.max(3,Math.min(innerHeight-27,y-4))}px)`;agentCursor.classList.add('visible');if(element){element.classList.add('agent-flash');setTimeout(()=>element.classList.remove('agent-flash'),1600)}}
function agentInputBusy(){
  return !!(drag||curveDrag||arrangePointer||clipDrag||trackOrderDrag||document.querySelector('dialog:modal')||
    document.activeElement?.matches('input,textarea,select,[contenteditable=true]'));
}
function agentRegions(event){
  if(event.regions?.length)return event.regions;
  if(event.beat_range)return [{track_id:event.track_id||trackId,beat_range:event.beat_range}];
  const selectedIds=new Set(event.note_ids||[]);
  return project.tracks.flatMap(track=>track.notes.filter(note=>selectedIds.has(note.id))
    .map(note=>({track_id:track.id,beat_range:[note.start,note.start+note.duration]})));
}
function agentLine(phase,action,outcome=''){
  const labels={idle:'Agent 待命',intent:'准备',thinking:'思考中',reading:'分析中',editing:'修改中',working:'处理中',rendering:'渲染中',review:'待确认',done:'完成',failed:'失败',listening:'试听中'};
  const label=labels[phase]||labels.done;
  if(activityBar.dataset.agentPhase===phase&&$('agentAction').textContent===action&&
    $('agentOutcome').textContent===outcome)return;
  activityBar.dataset.agentPhase=phase;
  $('agentPhase').textContent=label;
  $('agentAction').textContent=action;
  $('agentAction').title=action;
  $('agentOutcome').hidden=!outcome;
  $('agentOutcome').textContent=outcome;
  $('agentOutcome').title=outcome;
}
function resetAgentPresence(){
  focusGeneration++;
  agentRender=null;
  lastAgentOrder=0;
  showingAgentIntent=false;
  pendingAgentTarget=null;
  agentCursor.classList.remove('visible');
  agentLine('idle','');
}
function updateAgentRender(job,description){
  const tracked=agentRender;
  if(!tracked||job.job_id!==tracked.id||job.project_id!==project?.id||tracked.order<lastAgentOrder)return;
  if(['complete','failed','cancelled','interrupted'].includes(job.status)){
    agentRender=null;
    agentLine(job.status==='complete'||job.status==='cancelled'?'done':'failed',
      tracked.intent||description,tracked.intent?description:'');
  }else{
    const progress=`${description} · ${Math.round(job.progress*100||0)}%`;
    agentLine('rendering',tracked.intent||progress,tracked.intent?progress:'');
  }
}
for(const type of ['pause','ended'])audio.addEventListener(type,()=>{
  if(activityBar.dataset.agentPhase==='listening')agentLine('done',type==='ended'?'选段试听结束':'已停止试听');
});
const passiveAgentReads=new Set(['project_read','selection_resolve','timeline_query','arrangement_query','audio_clips']);
function displayAgentEvent(event){
  if(event.project_id&&event.project_id!==project?.id)return false;
  const intentOrder=event.kind==='focus'?event.sequence:event.intent_sequence;
  const linkedRender=['render_finished','render_cancel'].includes(event.kind)&&agentRender?.id===event.job_id?agentRender:null;
  const order=intentOrder||linkedRender?.order||event.sequence;
  if(order<lastAgentOrder)return false;
  if(!intentOrder&&showingAgentIntent&&passiveAgentReads.has(event.operation||event.kind)&&event.phase!=='failed')return false;
  lastAgentOrder=order;
  const intent=event.intent_reason||linkedRender?.intent;
  showingAgentIntent=!!(intentOrder||intent);
  const title=event.reason||'检查工程';
  if(event.kind==='render_start'&&event.job_id)agentRender={id:event.job_id,intent:event.intent_reason,order};
  if(event.kind==='render_finished'&&event.job_id===agentRender?.id)agentRender=null;
  const phase=event.phase||(event.kind==='focus'?'intent':event.kind==='desktop_transport'&&event.transport_action==='play'&&!audio.paused?'listening':event.status==='failed'?'failed':'done');
  const action=event.kind==='desktop_transport'&&event.transport_action==='play'&&audio.paused?'已发起选段试听':title;
  agentLine(phase,intent||action,intent?(event.outcome||title):'');
  return true;
}
async function displayAgentTarget(event){
  const generation=++focusGeneration;
  if(!project||event.project_id!==project.id||agentInputBusy())return;
  if(event.kind==='desktop_transport'&&event.revision!==project.revision)return;
  try{
    if(event.kind!=='desktop_transport'){
      const latest=await api('project_read',{project_id:project.id});
      if(generation!==focusGeneration||latest.id!==project?.id||agentInputBusy())return;
      if(latest.revision>project.revision)accept(latest);
      const regions=agentRegions(event);
      if(regions.length){await focusArrangement({...event,regions},generation);return}
      if(event.region==='arrangement'&&event.track_id){
        await focusArrangement(event,generation);
        return;
      }
    }
    const toolbarTarget={
      transport:$(event.transport_action==='stop'?'stop':event.kind==='project_export'?'exportProject':'play'),
      timeline:$('showTimeline'),composition:$('showDesign'),mixer:typeof mixerFocusTarget==='function'&&mixerFocusTarget(event.track_id)||$('trackTitle'),effects:$('showEffects')
    }[event.region]||$('trackTitle');
    if(!followAgent)return;
    arrangeFocus=null;
    queueArrangementDraw();
    const r=toolbarTarget.getBoundingClientRect();
    if(r.width&&r.height)showCursor(r.left+r.width*.65,r.top+r.height*.5,toolbarTarget);
  }catch{}
}



const agentTargetKinds=new Set(['focus','project_edit','project_undo','project_redo','audio_import','arrangement_edit','audio_clip_edit','audio_clip_group','selection_edit','render_start','project_export','desktop_transport']);
async function pollAgent(){
  try{
    const v=await api('agent_events',{after:eventSequence});
    if(journalReady&&v.sequence<eventSequence)resetAgentPresence();
    eventSequence=v.sequence;
    if(!journalReady){journalReady=true;return}
    const visible=v.events.filter(event=>!event.project_id||event.project_id===project?.id);
    let target;
    for(const event of visible){
      if(displayAgentEvent(event)&&agentTargetKinds.has(event.kind))target=event;
    }
    if(target)pendingAgentTarget={target,expires:performance.now()+5000};
    if(pendingAgentTarget?.expires<performance.now())pendingAgentTarget=null;
    if(pendingAgentTarget&&!agentInputBusy()){
      const next=pendingAgentTarget.target;
      pendingAgentTarget=null;
      await displayAgentTarget(next);
    }
  }catch{}finally{setTimeout(pollAgent,500)}
}
const knowledgeDialog=document.createElement('dialog');knowledgeDialog.id='knowledgeDialog';knowledgeDialog.className='knowledge-dialog';knowledgeDialog.innerHTML='<div class="dialog-heading"><h2 id="knowledgeTitle">创作知识</h2><button id="closeKnowledge" class="ghost" aria-label="关闭创作知识">×</button></div><div id="knowledgeFilters"><select id="knowledgeTopic" aria-label="知识主题"><option value="">全部主题</option><option value="melody">旋律与动机</option><option value="harmony">和声</option><option value="rhythm">节奏与律动</option><option value="prosody">歌词与演唱</option><option value="form">曲式</option><option value="arrangement">配器</option><option value="mix">混音</option><option value="style_blending">风格混搭</option></select><select id="knowledgeStyle" aria-label="风格指南"><option value="">选择风格</option></select></div><div id="knowledgeBody"></div>';document.body.append(knowledgeDialog);$('closeKnowledge').onclick=()=>knowledgeDialog.close();
function list(items){return '<ul>'+items.map(s=>'<li>'+esc(s)+'</li>').join('')+'</ul>'}
$('knowledgeTopic').add(new Option('流行歌制作与返修','pop_production'),1);
async function showKnowledge(){try{$('knowledgeTitle').textContent='创作知识 · 有来源，可复核';$('knowledgeFilters').hidden=false;const v=await api('music_knowledge',{...($('knowledgeTopic').value?{topic:$('knowledgeTopic').value}:{}),...($('knowledgeStyle').value?{style:$('knowledgeStyle').value}:{})});if($('knowledgeStyle').options.length===1){const index=await api('music_knowledge',{topic:'style_blending'});for(const s of index.styles)$('knowledgeStyle').add(new Option(s.name,s.id))}
  $('knowledgeBody').innerHTML='<p class="knowledge-scope">'+esc(v.scope)+'</p>'+v.styles.filter(s=>s.focus).map(s=>'<section class="knowledge-card"><h3>'+esc(s.name)+'</h3>'+list(s.focus)+'<h4>配器职责</h4>'+list(s.roles)+'<h4>常见偏差</h4>'+list(s.pitfalls)+'</section>').join('')+v.topics.map(t=>'<section class="knowledge-card"><h3>'+esc(t.name)+'</h3>'+list(t.principles)+'<h4>试听要回答</h4>'+list(t.listen_for)+'</section>').join('')+'<section class="knowledge-card"><h3>从设计到交付</h3>'+v.workflow.map((w,i)=>'<div class="workflow-row"><b>'+String(i+1).padStart(2,'0')+'</b><div><strong>'+esc(w.stage)+'</strong><p>'+esc(w.deliverable)+'</p><small>'+esc(w.check)+'</small></div></div>').join('')+'</section><p class="knowledge-sources">资料：'+Object.values(v.sources).filter(s=>s.url).map(s=>'<a target="_blank" rel="noreferrer" href="'+esc(s.url)+'">'+esc(s.title)+'</a>').join(' · ')+'</p>';if(!knowledgeDialog.open)knowledgeDialog.showModal()
}catch(e){toast(e.message,true)}}
$('showKnowledge').onclick=showKnowledge;$('knowledgeTopic').onchange=$('knowledgeStyle').onchange=showKnowledge;
$('showDesign').onclick=()=>{$('knowledgeTitle').textContent='作品设计';$('knowledgeFilters').hidden=true;const p=project?.composition;$('knowledgeBody').innerHTML=p?'<p class="knowledge-scope">'+(p.purpose==='technical_test'?'工程功能测试，不作为成品歌曲。':'先明确作品的选择，再逐段试听与修订。')+'</p><section class="knowledge-card"><h3>创作意图</h3><p>'+esc(p.brief)+'</p>'+['style','key','motif','emotional_arc','lyric_language'].filter(k=>p[k]).map(k=>'<p><b>'+esc({style:'风格',key:'调性',motif:'动机',emotional_arc:'情绪变化',lyric_language:'歌词语言'}[k])+ '</b>　'+esc(p[k])+'</p>').join('')+'</section>'+((p.sections||[]).length?'<section class="knowledge-card"><h3>段落设计</h3>'+p.sections.map(s=>'<p><b>'+esc(s.name)+'</b>　'+s.start+'—'+s.end+' 拍<br><small>'+esc(s.role)+'</small></p>').join('')+'</section>':'')+((p.arrangement||[]).length?'<section class="knowledge-card"><h3>配器职责</h3>'+list(p.arrangement.map(a=>typeof a==='string'?a:JSON.stringify(a)))+'</section>':''):'<section class="knowledge-card"><h3>还没有作品设计</h3><p>先确定具体主题、核心动机、情绪变化与段落职责，再开始写一首新歌。Agent 会把创作选择保存在这里。</p></section>';knowledgeDialog.showModal()};
function reviewBeatText(range){return range.map(beat=>Number(beat.toFixed(3))).join('–')+' 拍'}

function locateReviewPhrase(phrase,binding,syllable=null){
  const focus=async()=>{
    if(project?.id!==binding.project_id||project.revision!==binding.expected_revision){
      toast('工程已更新，请重新复核',true);
      return;
    }
    const row=project.tracks.findIndex(track=>track.id===phrase.track_id);
    if(row<0)return;
    const [start,end]=(syllable||phrase).beat_range;
    cursorBeat=syllable?.beat??start;
    trackId=phrase.track_id;
    if(syllable){
      updateArrangeSelection(null);
      chooseGroup(syllable.note_ids);
      await switchArrangement(false);
      if(!motifBindingCurrent(binding))return;
      const first=picked()[0];
      const viewport=$('rollViewport');
      viewport.tabIndex=-1;
      viewport.focus({preventScroll:true});
      if(first)viewport.scrollTo(Math.max(0,beatX(cursorBeat)-keyW-32),
        Math.max(0,pitchY(first.pitch)-viewport.clientHeight/2));
      refresh();
      return;
    }
    chooseGroup([]);
    updateArrangeSelection({start,end,ids:[phrase.track_id]});
    void switchArrangement(true);
    arrangeViewport.scrollTo(Math.max(0,start*arrangeZoom-45),Math.max(0,(row-1)*arrangeRow));
    refresh();
  };
  if(knowledgeDialog.open){
    knowledgeDialog.addEventListener('close',focus,{once:true});
    knowledgeDialog.close();
  }else focus();
}

function scoreListeningPoints(phrase,index,canAudition){
  const labels={entry:'首字',longest_syllable:'长音',pitch_peak:'最高音',release:'句尾'};
  const describe=attack=>{
    if(!attack)return '';
    const tracks=attack.tracks.slice(0,2).map(track=>track.track_name).join(' / ')+
      (attack.tracks.length>2?' 等':'');
    const when=attack.offset_ms===0?'同拍':`${attack.offset_ms<0?'前':'后'} ${Math.abs(attack.offset_ms)} ms`;
    return `${when} · ${tracks}`;
  };
  const points=(phrase.listening_points||[]).map((point,at)=>{
    const roles=point.roles.map(role=>labels[role]).join(' / ');
    const beat=Number(point.beat.toFixed(3))+' 拍';
    const before=describe(point.backing.before),after=describe(point.backing.on_or_after);
    const label=[roles,point.lyric,beat].filter(Boolean).join(' · ');
    const detail=[label,noteName(point.pitch),before,after].filter(Boolean).join(' · ');
    const playLabel='试听 '+roles+' · '+beat;
    return `<div class="score-listening-item"><button class="score-listening-point" data-listening-phrase="${index}" data-listening-index="${at}"
      aria-label="${esc(detail)}" title="${esc(detail)}"><b>${esc(label)}</b>${after||before?'<small>'+esc(after||before)+'</small>':''}</button>
      ${canAudition&&point.audition_range?`<button class="score-listening-play" data-audition-phrase="${index}" data-audition-point="${at}"
        aria-label="${esc(playLabel)}" title="${esc(playLabel+' · '+reviewBeatText(point.audition_range))}">${icon('play')}</button>`:''}</div>`;
  }).join('');
  return points?'<div class="score-listening-points">'+points+'</div>':'';
}

let reviewPair=null,reviewPreviewEpoch=0;

async function releaseReviewPair(){
  reviewPreviewEpoch++;
  const old=reviewPair;
  reviewPair=null;
  if(!old)return;
  await audio.stopAudition(old.id).catch(()=>{});
  await api('range_audition',{action:'release',...old.binding,audition_id:old.id}).catch(()=>{});
}
knowledgeDialog.addEventListener('close',()=>void releaseReviewPair());

function motifBindingCurrent(binding){
  return project?.id===binding.project_id&&project.revision===binding.expected_revision;
}

function reviewAuditionControls(labelA,labelB,matchLevels=true,showLevels=false){
  return '<div class="motif-audition" hidden><button data-motif-side="A" aria-pressed="false">'+labelA+
    '</button><button data-motif-side="B" aria-pressed="false">'+labelB+
    '</button><button data-motif-stop aria-label="停止试听">'+icon('stop')+
    '</button><select aria-label="试听响度"><option value="lufs"'+(matchLevels?' selected':'')+
    '>匹配响度</option><option value="original"'+(!matchLevels?' selected':'')+
    '>原电平</option></select>'+
    (showLevels?'<button data-review-levels aria-expanded="false" aria-controls="scorePhraseLevels">电平</button>':'')+
    '<button data-review-export disabled>导出 A/B</button>'+
    '<output role="status"></output></div>';
}

function wireReviewAudition(controls,binding,comparison,onPair){
  const status=controls.querySelector('output');
  const match=controls.querySelector('select');
  const buttons=[...controls.querySelectorAll('[data-motif-side]')];
  const stop=controls.querySelector('[data-motif-stop]');
  const exportButton=controls.querySelector('[data-review-export]');
  const levelsButton=controls.querySelector('[data-review-levels]');
  const levelsPanel=levelsButton&&$('scorePhraseLevels');
  let row=null,prepared=null,busy=false,window=null;
  function closeLevels(){
    if(!levelsPanel)return;
    levelsPanel.hidden=true;
    levelsButton.setAttribute('aria-expanded','false');
  }
  function highlightLevelWindow(){
    for(const button of levelsPanel?.querySelectorAll('[data-level-window]')||[])
      button.setAttribute('aria-pressed',String(button.dataset.levelWindow===window?.join(',')));
  }
  const paint=()=>{
    const active=prepared&&audio.auditionId===prepared.id&&!audio.paused;
    buttons.forEach((button,index)=>button.setAttribute('aria-pressed',String(!!active&&audio.auditionVariant===index)));
    stop.disabled=!active;
  };
  const changed=()=>{if(controls.isConnected)paint();else audio.removeEventListener('timeupdate',changed)};
  audio.addEventListener('timeupdate',changed);
  knowledgeDialog.addEventListener('close',()=>audio.removeEventListener('timeupdate',changed),{once:true});
  const play=async variant=>{
    if(!prepared||!motifBindingCurrent(binding))throw new Error('工程已更新，请重新复核');
    if(reviewPair?.id!==prepared.id)throw new Error('试听已结束，请重新选择');
    await audio.auditionSide(prepared.id,variant);
    paint();
  };
  const prepare=async(index,selectedWindow=null)=>{
    if(busy)return;
    const options=comparison(index,selectedWindow);
    if(index!==row||levelsButton?.disabled)closeLevels();
    busy=true;row=index;window=selectedWindow;prepared=null;
    onPair?.(null);
    highlightLevelWindow();
    controls.hidden=false;match.disabled=true;buttons.forEach(button=>button.disabled=true);stop.disabled=true;
    exportButton.disabled=true;
    if(levelsButton)levelsButton.disabled=true;
    if(options.outputLabel)buttons[1].textContent=options.outputLabel;
    status.textContent='准备两段试听…';
    controls.scrollIntoView({block:'nearest'});
    const releasing=releaseReviewPair();
    const generation=reviewPreviewEpoch;
    try{
      await releasing;
      if(!motifBindingCurrent(binding))throw new Error('工程已更新，请重新复核');
      const engine=$('renderEngine').value;
      const isCurrentMix=mix=>mix?.project_id===binding.project_id&&mix.revision===binding.expected_revision&&mix.engine===engine;
      let mix=isCurrentMix(rendered)?rendered:null;
      if(!mix){
        const {jobs}=await api('render_jobs',{...binding,engine});
        mix=jobs.find(job=>isCurrentMix(job.result))?.result||null;
      }
      if(!mix){
        if(rendering)throw new Error('混音正在渲染');
        if(!knowledgeDialog.open||!controls.isConnected||generation!==reviewPreviewEpoch||!motifBindingCurrent(binding))return;
        await render(false);
        mix=rendered;
      }
      if(!isCurrentMix(mix)||!motifBindingCurrent(binding))throw new Error('需要本版本的混音');
      if(!knowledgeDialog.open||!controls.isConnected||generation!==reviewPreviewEpoch)return;
      const result=await api('range_audition',{action:'prepare',...binding,job_id:mix.job_id,
        ...options.args,level_match:match.value==='lufs'});
      if(!knowledgeDialog.open||!controls.isConnected||generation!==reviewPreviewEpoch||!motifBindingCurrent(binding)){
        await api('range_audition',{action:'release',...binding,audition_id:result.id}).catch(()=>{});
        return;
      }
      prepared=result;reviewPair={id:result.id,binding};
      onPair?.({pair:result,index});
      if(levelsButton){
        if(levelsPanel.dataset.job!==result.saved.job_id)closeLevels();
        levelsButton.disabled=false;
      }
      buttons[0].title=reviewBeatText(result.beat_ranges[0]);
      buttons[1].title=reviewBeatText(result.beat_ranges[1]);
      const level=result.matched_lufs?' · LUFS':result.matched_rms?' · RMS':'';
      const ranges=result.beat_ranges.map(reviewBeatText);
      status.textContent=(ranges[0]===ranges[1]?ranges[0]:ranges.join(' / '))+level;
      buttons.forEach(button=>button.disabled=false);
      exportButton.disabled=false;
      await play('A');
    }catch(error){if(controls.isConnected&&knowledgeDialog.open)status.textContent=error.message}
    finally{busy=false;match.disabled=false}
  };
  buttons.forEach(button=>button.onclick=()=>void play(button.dataset.motifSide).catch(error=>{status.textContent=error.message}));
  stop.onclick=()=>{if(prepared)void audio.stopAudition(prepared.id).catch(error=>{status.textContent=error.message})};
  exportButton.onclick=async()=>{
    const pair=prepared;
    if(!pair||reviewPair?.id!==pair.id||!motifBindingCurrent(binding))return;
    exportButton.disabled=true;
    try{
      const result=await api('range_audition',{action:'export',...binding,audition_id:pair.id});
      if(prepared===pair&&reviewPair?.id===pair.id&&knowledgeDialog.open)await download(result.url);
    }catch(error){if(prepared===pair&&knowledgeDialog.open)status.textContent=error.message}
    finally{if(prepared===pair)exportButton.disabled=false}
  };
  match.onchange=()=>{if(row!==null)void prepare(row,window)};
  if(levelsButton)levelsButton.onclick=async()=>{
    if(!levelsPanel.hidden){closeLevels();return}
    if(!prepared||!motifBindingCurrent(binding)||reviewPair?.id!==prepared.id)return;
    const current=prepared;
    const index=row;
    try{
      levelsButton.disabled=true;
      levelsPanel.hidden=false;
      levelsButton.setAttribute('aria-expanded','true');
      levelsPanel.textContent='测量原电平…';
      const {beat_ranges,outputs}=comparison(index).args;
      const result=await api('render_balance',{...binding,job_id:current.saved.job_id,
        request:{beat_range:beat_ranges[0],focus:outputs[1],include_remainder:true}});
      if(prepared!==current||reviewPair?.id!==current.id||!levelsPanel.isConnected)return;
      levelsPanel.dataset.job=current.saved.job_id;
      const heading=document.createElement('div');
      heading.textContent=(outputs[1][0].kind==='group'?'编组':'人声轨')+' / 其余混音 · 1–4 kHz · 原电平';
      const strip=document.createElement('div');strip.className='phrase-level-strip';
      const height=db=>Number.isFinite(db)?Math.max(2,Math.min(100,100*(db+60)/60))+'%':'0%';
      const value=db=>Number.isFinite(db)?db.toFixed(1)+' dBFS':'静音';
      for(const item of result.windows){
        const button=document.createElement('button');
        button.dataset.levelWindow=item.beat_range.join(',');
        button.title=reviewBeatText(item.beat_range)+' · '+value(item.focus.band_rms_dbfs)+' / '+value(item.reference.band_rms_dbfs);
        button.setAttribute('aria-label','试听 '+button.title);
        for(const levels of [item.focus,item.reference]){
          const bar=document.createElement('span');
          bar.style.height=height(levels.band_rms_dbfs);
          button.append(bar);
        }
        button.onclick=()=>void prepare(index,item.beat_range);
        strip.append(button);
      }
      levelsPanel.replaceChildren(heading,strip);
      highlightLevelWindow();
      animateElement(levelsPanel,[{opacity:0},{opacity:1}],'insert');
    }catch(error){if(prepared===current&&levelsPanel.isConnected)levelsPanel.textContent=error.message}
    finally{if(prepared===current)levelsButton.disabled=false}
  };
  return prepare;
}

function renderMotifReview(panel,result,binding,onWholeSong){
  const entries=[result.anchor,...result.returns];
  const label=(row,index)=>{
    if(index===0)return '参照动机';
    const fields=[row.relation==='changed_ending'?'句尾变化':'再现'];
    if(row.semitone_shift)fields.push((row.semitone_shift>0?'+':'')+row.semitone_shift+' 半音');
    if(row.beat_scale!==1)fields.push('节奏 ×'+row.beat_scale);
    if(row.same_lyric_tokens===false)fields.push('换词');
    if(row.same_lyric_placement===false)fields.push('落字变化');
    return fields.join(' · ');
  };
  panel.innerHTML='<header class="motif-review-heading"><h3>动机回归</h3>'+
    (onWholeSong?'<button class="ghost" id="motifWholeSong">查全曲</button>':'')+'</header>'+
    (window.desktopNative&&result.returns.length?reviewAuditionControls('A 参照','B 回归'):'')+entries.map((row,index)=>{
    const chords=(row.harmony_spans||[]).map(item=>esc(item.symbol)).join(' · ');
    const backing=(row.backing_attacks||[]).map(item=>esc(item.name)+' '+item.attacks).join(' · ');
    return `<div class="motif-result">
      <header><strong>${label(row,index)}</strong><span>${reviewBeatText(row.beat_range)}${row.section?' · '+esc(row.section):''}</span>
      <span class="motif-row-actions">
      ${index&&window.desktopNative?'<button class="ghost" data-motif-audition="'+index+'">试听</button>':''}
      <button class="ghost" data-motif-location="${index}">定位</button></span></header>
      <p>${esc(row.lyrics.join(' '))}</p>
      ${chords||backing?'<small>'+[chords,backing].filter(Boolean).join('　/　')+'</small>':''}
    </div>`;
  }).join('')+(result.total_returns===0?'<p class="knowledge-scope">当前范围内没有找到回归候选。</p>':'')+
    (result.truncated?'<p class="knowledge-scope">显示 '+result.returns.length+' / '+result.total_returns+' 处</p>':'');
  for(const button of panel.querySelectorAll('[data-motif-location]')){
    button.onclick=()=>locateReviewPhrase(entries[Number(button.dataset.motifLocation)],binding);
  }
  if(onWholeSong)$('motifWholeSong').onclick=onWholeSong;
  const controls=panel.querySelector('.motif-audition');
  if(controls){
    const prepare=wireReviewAudition(controls,binding,index=>({args:{beat_ranges:[entries[0].beat_range,entries[index].beat_range]}}));
    for(const button of panel.querySelectorAll('[data-motif-audition]'))
      button.onclick=()=>void prepare(Number(button.dataset.motifAudition));
  }
  panel.hidden=false;
  animateElement(panel,[{opacity:0,transform:'translateY(4px)'},{opacity:1,transform:'translateY(0)'}],'insert');
  panel.scrollIntoView({block:'nearest'});
}

async function showSectionContrast(binding,defaultRange){
  if(!motifBindingCurrent(binding))return;
  await releaseReviewPair();
  if(!knowledgeDialog.open||!motifBindingCurrent(binding))return;
  $('knowledgeTitle').textContent='段落对照';
  const body=$('knowledgeBody');
  const parts=arrangementModel().sections;
  const singers=project.tracks.filter(track=>!track.instrument&&!isAudioTrack(track)&&track.notes.length);
  body.innerHTML='<button id="sectionBack" class="ghost">‹ 曲谱复核</button>'+
    '<form id="sectionContrastForm">'+['A','B'].map(side=>`<div class="section-compare-pair">
      <strong>${side}</strong><select id="sectionPart${side}" aria-label="${side} 段落"><option value="">自定拍段</option></select>
      <input id="sectionFrom${side}" type="number" min="0" max="50000" step="any" required aria-label="${side} 起拍">
      <input id="sectionTo${side}" type="number" min="0" max="50000" step="any" required aria-label="${side} 止拍"></div>`).join('')+
    (singers.length>1?'<label>主唱 <select id="sectionLead" aria-label="对照主唱"></select></label>':'')+
    '<button id="sectionCompare" type="submit" class="primary">对照</button><output id="sectionCompareState" role="status"></output></form>'+
    '<section id="sectionContrastResult"></section>';
  $('sectionBack').onclick=()=>$('reviewSong').click();
  if(singers.length>1)$('sectionLead').replaceChildren(new Option('选择主唱',''),...singers.map(track=>new Option(track.name,track.id)));
  const mid=(defaultRange[0]+defaultRange[1])/2;
  for(const [index,side] of ['A','B'].entries()){
    const select=$('sectionPart'+side),from=$('sectionFrom'+side),to=$('sectionTo'+side);
    select.hidden=!parts.length;
    select.append(...parts.map((part,i)=>new Option(part.name,String(i))));
    select.onchange=()=>{
      const part=select.value===''?null:parts[Number(select.value)];
      if(part){from.value=part.start;to.value=part.end}
    };
    for(const input of [from,to])input.oninput=()=>{select.value=''};
    if(parts.length>=2){select.value=String(index);select.onchange()}
    else [from.value,to.value]=index?[mid,defaultRange[1]]:[defaultRange[0],mid];
  }
  const form=$('sectionContrastForm'),result=$('sectionContrastResult'),status=$('sectionCompareState');
  let ticket=0;
  form.onsubmit=async event=>{
    event.preventDefault();
    if(!motifBindingCurrent(binding)){status.textContent='工程已更新，请重新打开对照';return}
    const version=++ticket,button=$('sectionCompare');
    const beat_ranges=['A','B'].map(side=>[$('sectionFrom'+side).valueAsNumber,$('sectionTo'+side).valueAsNumber]);
    if(beat_ranges.some(([from,to])=>!Number.isFinite(from)||!Number.isFinite(to)||to<=from)){
      status.textContent='请选择有效的起拍和止拍';return;
    }
    status.textContent='';button.disabled=true;
    try{
      await releaseReviewPair();
      const lead_track_id=$('sectionLead')?.value;
      const compared=await api('section_contrast',{...binding,beat_ranges,...(lead_track_id?{lead_track_id}:{})});
      if(version!==ticket||!form.isConnected||!knowledgeDialog.open||!motifBindingCurrent(binding))return;
      renderSectionContrast(result,compared,binding);
    }catch(error){if(form.isConnected)status.textContent=error.message}
    finally{if(form.isConnected&&version===ticket)button.disabled=false}
  };
  form.requestSubmit();
}

function renderSectionContrast(panel,detail,binding){
  const labels=detail.sections.map((row,i)=>row.name||['段落 A','段落 B'][i]);
  const cueLabels={section_opening:'段首对照',enters:'进入 B',drops:'在 B 退出',release_response:'句尾应答',
    similar_attacks:'近似律动',lead_entry:'主唱首字',lead_release:'主唱尾音'};
  const ratio=value=>value==null?'—':Math.round(value*100)+'%';
  const count=side=>side.clip_count?side.clip_seconds+' 秒素材':String(side.attack_count);
  const rows=detail.tracks.map(track=>{
    const [a,b]=track.sides;
    const overlap=a.register_overlap_seconds==null?'':`<small>同音区 ${a.register_overlap_seconds} → ${b.register_overlap_seconds} 秒</small>`;
    const gaps=a.in_lead_gaps_attacks==null?'':`<small>空隙入音 ${a.in_lead_gaps_attacks} → ${b.in_lead_gaps_attacks}</small>`;
    return `<tr data-section-track="${esc(track.track_id)}"><th scope="row">${esc(track.track_name)}${track.enabled_in_mix?'':'<small>未进入混音</small>'}${gaps}${overlap}</th>
      <td>${esc(count(a))}</td><td>${esc(count(b))}</td><td>${ratio(track.drum_hit_similarity??track.onset_similarity)}</td></tr>`;
  }).join('');
  panel.innerHTML='<div class="section-compare-summary">'+detail.sections.map((row,i)=>
    `<div><strong>${esc(labels[i])}</strong><span>${reviewBeatText(row.beat_range)} · ${row.enabled_tracks_with_material} 轨${row.lead_gap_seconds==null?'':' · 主唱留白 '+row.lead_gap_seconds+' 秒'}</span></div>`).join('')+'</div>'+
    '<table class="section-compare-table"><thead><tr><th>音轨</th><th>A 入音</th><th>B 入音</th><th>律动重合</th></tr></thead><tbody>'+rows+'</tbody></table>'+
    (window.desktopNative?'<div class="section-compare-cues">'+detail.listening_points.map((cue,i)=>
      `<button data-section-cue="${i}" data-section-kind="${esc(cue.kind)}" title="${esc(cue.beat_ranges.map(reviewBeatText).join(' / '))}">${icon('play')}${esc([cue.track_name,cueLabels[cue.kind]].filter(Boolean).join(' · '))}</button>`).join('')+
      '</div>'+reviewAuditionControls('A '+esc(labels[0]),'B '+esc(labels[1])):'');
  const controls=panel.querySelector('.motif-audition');
  if(controls){
    const measure=document.createElement('button');
    measure.type='button';measure.textContent='原电平';measure.dataset.sectionMeasure='';
    measure.setAttribute('aria-expanded','false');
    const proof=document.createElement('div');
    proof.className='section-audio-proof';proof.hidden=true;proof.setAttribute('role','status');
    controls.querySelector('output').before(measure);
    controls.after(proof);
    let active=null;
    const showProof=show=>{
      proof.hidden=!show;
      measure.setAttribute('aria-expanded',String(show));
    };
    const prepare=wireReviewAudition(controls,binding,index=>({args:{beat_ranges:detail.listening_points[index].beat_ranges}}),
      state=>{
        active=state;measure.disabled=!state;
        proof.replaceChildren();showProof(false);
      });
    measure.disabled=true;
    measure.onclick=async()=>{
      if(!proof.hidden){showProof(false);return}
      const selected=active;
      const current=()=>selected&&active===selected&&reviewPair?.id===selected.pair.id&&proof.isConnected&&
        knowledgeDialog.open&&motifBindingCurrent(binding);
      if(!current())return;
      showProof(true);measure.disabled=true;proof.textContent='测量原电平…';
      try{
        const report=await api('section_contrast',{...binding,job_id:selected.pair.saved.job_id,audio_cue:selected.index,
          beat_ranges:detail.sections.map(section=>section.beat_range),
          ...(detail.lead_track_id?{lead_track_id:detail.lead_track_id}:{})});
        if(!current())return;
        const format=(value,unit)=>Number.isFinite(value)?value.toFixed(1)+' '+unit:'—';
        const fields=[
          ['响度', report.audio.sides.map(side=>format(side.integrated_lufs,'LUFS'))],
          ['宽带 RMS', report.audio.sides.map(side=>format(side.mix.rms_dbfs,'dBFS'))],
        ];
        if(report.audio.lead_track_id)
          fields.push(['主唱相对其余混音 · 1–4 kHz',report.audio.sides.map(side=>
            format(side.lead_balance.focus_minus_reference_band_db,'dB'))]);
        proof.innerHTML=fields.map(([label,[a,b]])=>
          `<div><span>${esc(label)}</span><b>${esc(a)}</b><b>${esc(b)}</b></div>`).join('');
      }catch(error){if(current())proof.textContent=error.message}
      finally{if(active===selected)measure.disabled=false}
    };
    for(const button of panel.querySelectorAll('[data-section-cue]'))button.onclick=()=>void prepare(Number(button.dataset.sectionCue));
  }
}

async function showRhythmInterplay(binding,range,preferred={}){
  await releaseReviewPair();
  if(!knowledgeDialog.open||!motifBindingCurrent(binding))return;
  $('knowledgeTitle').textContent='鼓贝斯';
  const body=$('knowledgeBody');
  const drums=project.tracks.filter(track=>track.instrument?.drums);
  const bass=project.tracks.filter(track=>track.instrument&&!track.instrument.drums);
  body.innerHTML='<button id="rhythmBack" class="ghost">‹ 曲谱复核</button>';
  $('rhythmBack').onclick=()=>$('reviewSong').click();
  if(!drums.length||!bass.length){
    const message=document.createElement('p');message.textContent='需要一条鼓组轨和一条有音乐器轨';
    body.append(message);return;
  }
  body.insertAdjacentHTML('beforeend',`<form id="rhythmForm" class="rhythm-controls">
    <label>鼓组 <select id="rhythmDrum"></select></label><label>贝斯 <select id="rhythmBass"></select></label>
    <label>起拍 <input id="rhythmFrom" type="number" min="0" max="50000" step="any" required></label>
    <label>止拍 <input id="rhythmTo" type="number" min="0" max="50000" step="any" required></label>
    <button id="rhythmCompare" type="submit" class="primary">复核</button>
    <output id="rhythmState" role="status"></output></form><section id="rhythmResult"></section>`);
  const form=$('rhythmForm'),panel=$('rhythmResult');
  for(const [id,tracks] of [['rhythmDrum',drums],['rhythmBass',bass]])
    $(id).replaceChildren(...tracks.map(track=>new Option(track.name,track.id)));
  $('rhythmDrum').value=preferred.drum_track_id||drums[0].id;
  $('rhythmBass').value=preferred.bass_track_id||(bass.find(track=>track.instrument.program>=32&&track.instrument.program<=39)||bass[0]).id;
  $('rhythmFrom').value=range[0];$('rhythmTo').value=range[1];
  let ticket=0;
  form.oninput=()=>{
    ticket++;
    $('rhythmCompare').disabled=false;
    panel.replaceChildren();
    void releaseReviewPair();
  };
  form.onsubmit=async event=>{
    event.preventDefault();
    const request={...binding,beat_range:[$('rhythmFrom').valueAsNumber,$('rhythmTo').valueAsNumber],
      drum_track_id:$('rhythmDrum').value,bass_track_id:$('rhythmBass').value};
    const button=$('rhythmCompare'),status=$('rhythmState'),version=++ticket;
    if(request.beat_range[1]<=request.beat_range[0]){status.textContent='止拍须晚于起拍';return}
    status.textContent='';button.disabled=true;
    try{
      await releaseReviewPair();
      const report=await api('rhythm_interplay',request);
      if(form.isConnected&&knowledgeDialog.open&&version===ticket&&motifBindingCurrent(binding))
        renderRhythmInterplay(panel,report,binding,request);
    }catch(error){if(form.isConnected)status.textContent=error.message}
    finally{if(form.isConnected&&version===ticket)button.disabled=false}
  };
  form.requestSubmit();
}

function renderRhythmInterplay(panel,report,binding,request){
  const {summary,events}=report;
  const formatBeat=attack=>attack?Number(attack.beat.toFixed(3)):'—';
  const suffix=value=>value>0?'+'+value:String(value);
  const rows=events.map((event,index)=>`<tr>
    <td>${formatBeat(event.kick)}</td><td>${formatBeat(event.bass)}</td>
    <td>${event.offset_ms==null?(event.held_bass_at_kick?'延音':'—'):suffix(event.offset_ms)+' ms'}</td>
    <td>${event.kick&&event.bass?.notes.length===1?`<button data-rhythm-edit="${index}">调整</button>`:''}
    ${window.desktopNative?`<button data-rhythm-listen="${index}" aria-label="试听第 ${index+1} 个落点">${icon('play')}</button>`:''}</td></tr>`).join('');
  const mid=summary.median_offset_ms==null?'':' · 中值 '+suffix(summary.median_offset_ms)+' ms';
  panel.innerHTML='<p class="knowledge-scope">'+summary.paired+' 对相近落点 · '+summary.kick_only+' 次独立底鼓 · '+
    summary.bass_only+' 次独立贝斯'+mid+'</p>'+
    '<div id="rhythmPlacement" hidden></div>'+
    '<table class="section-compare-table"><thead><tr><th>底鼓拍位</th><th>贝斯拍位</th><th>贝斯相对底鼓</th><th></th></tr></thead><tbody>'+rows+'</tbody></table>'+
    (report.events_truncated?'<p class="knowledge-scope">已显示前 '+events.length+' 个落点，可缩小拍范围细查。</p>':'')+
    (window.desktopNative?reviewAuditionControls('A 鼓贝斯','B 全混',false):'');
  const controls=panel.querySelector('.motif-audition');
  if(controls){
    const ids=[request.drum_track_id,request.bass_track_id];
    const tracks=project.tracks.filter(track=>ids.includes(track.id));
    const solo=project.tracks.some(track=>track.solo&&!track.mute);
    const hasDirect=tracks.length===2&&tracks.every(track=>!track.output_group&&!track.mute&&(!solo||track.solo));
    if(!hasDirect){
      controls.querySelector('[data-motif-side="A"]').textContent='播放全混';
      controls.querySelector('[data-motif-side="B"]').hidden=true;
    }
    const prepare=wireReviewAudition(controls,binding,index=>({
      args:{beat_ranges:[events[index].listen_range,events[index].listen_range],
        ...(hasDirect?{outputs:[ids.map(id=>({kind:'track',id})),null]}:{})}}));
    for(const button of panel.querySelectorAll('[data-rhythm-listen]'))button.onclick=()=>void prepare(Number(button.dataset.rhythmListen));
  }
  for(const button of panel.querySelectorAll('[data-rhythm-edit]'))
    button.onclick=()=>showBassPlacement(events[Number(button.dataset.rhythmEdit)],binding,request);
}

function showBassPlacement(event,binding,request){
  const panel=$('rhythmPlacement');
  panel.hidden=false;
  panel.innerHTML=`<div class="rhythm-controls">
    <label>相对底鼓 · ms<input id="rhythmOffset" type="number" step="any" value="${event.offset_ms}"></label>
    <label>贝斯音长 · ms<input id="rhythmLength" type="number" min="0" step="any" placeholder="保留"></label>
    <button id="rhythmPreview" type="button">预览</button><button id="rhythmApply" type="button" class="primary" disabled>应用</button>
    <output id="rhythmPlacementState" role="status"></output></div>`;
  const offset=$('rhythmOffset'),length=$('rhythmLength'),state=$('rhythmPlacementState'),apply=$('rhythmApply');
  let draft=null,version=0;
  for(const input of [offset,length])input.oninput=()=>{version++;draft=null;apply.disabled=true;state.textContent=''};
  $('rhythmPreview').onclick=async()=>{
    const run=++version;
    draft=null;apply.disabled=true;
    const placement={kick_note_id:event.kick.notes[0].note_id,bass_note_id:event.bass.notes[0].note_id,
      offset_ms:offset.valueAsNumber,...(length.value?{length_ms:length.valueAsNumber}:{})};
    if(!Number.isFinite(placement.offset_ms)||(length.value&&!(placement.length_ms>0))){state.textContent='请输入有效的时差和音长';return}
    try{
      const preview=await api('rhythm_interplay',{...request,placement});
      if(run!==version||!apply.isConnected||!motifBindingCurrent(binding))return;
      draft=preview.placement;apply.disabled=false;
      state.textContent=reviewBeatText(draft.before_beat_range)+' → '+reviewBeatText(draft.after_beat_range);
    }catch(error){if(run===version&&state.isConnected)state.textContent=error.message}
  };
  apply.onclick=async()=>{
    const plan=draft;
    if(!plan||apply.disabled)return;
    apply.disabled=true;offset.disabled=length.disabled=$('rhythmPreview').disabled=true;
    try{
      await editChain;
      await releaseReviewPair();
      if(!motifBindingCurrent(binding))throw new Error('工程已更新，请重新复核');
      const saved=await api('project_edit',{...binding,operations:plan.operations});
      if(project?.id===saved.id)accept(saved);
      await refreshList();
      if(knowledgeDialog.open)await showRhythmInterplay({project_id:saved.id,expected_revision:saved.revision},request.beat_range,request);
    }catch(error){if(state.isConnected){state.textContent=error.message;apply.disabled=false;offset.disabled=length.disabled=$('rhythmPreview').disabled=false}}
  };
  panel.scrollIntoView({block:'nearest'});
}

$('reviewSong').onclick=async()=>{
  if(!project)return;
  const binding={project_id:project.id,expected_revision:project.revision};
  const range=workspaceFocus==='arrangement'&&arrangeSelection;
  try{
    const result=await api('composition_review',{...binding,
      ...(range?{beat_range:[range.start,range.end],track_ids:range.ids}:{})});
    if(project?.id!==binding.project_id||project.revision!==binding.expected_revision)throw new Error('工程已更新，请重新复核');
    const detail=result.score_detail;
    const written=detail.lyric_lines||[];
    $('knowledgeTitle').textContent='曲谱复核';
    $('knowledgeFilters').hidden=true;
    const body=$('knowledgeBody');
    const observation=(phrase)=>detail.observations.find(item=>item.track_id===phrase.track_id&&item.beat_range[0]===phrase.beat_range[0]);
    const outputs=detail.phrases.map(phrase=>{
      if(!window.desktopNative||!phrase.audible_in_mix)return null;
      const track=project.tracks.find(track=>track.id===phrase.track_id);
      if(!track?.output_group)return {kind:'track',id:phrase.track_id};
      const group=project.mix_groups?.find(group=>group.id===track.output_group);
      return group&&!group.mute?{kind:'group',id:group.id}:null;
    });
    const phrases=detail.phrases.map((phrase,index)=>{
      const gap=phrase.gap_after;
      const cue=observation(phrase);
      const words=(phrase.syllables||[]).map((syllable,at)=>{
        const text=syllable.lyric||'〜';
        const label=text+(syllable.lyric&&syllable.note_ids.length>1?'〜':'');
        const when=reviewBeatText(syllable.beat_range)+' · '+Number(syllable.notated_seconds.toFixed(3))+' 秒';
        return '<button class="score-syllable" data-syllable-phrase="'+index+'" data-syllable-index="'+at+
          '" title="'+esc(when)+'" aria-label="'+esc('定位 '+label+' · '+when)+'">'+esc(label)+'</button>';
      }).join(' ');
      return `<div class="score-review-phrase">
        <div><b>${esc(phrase.track_name)}</b><span>${reviewBeatText(phrase.beat_range)} · 第 ${phrase.bar.bar} 小节</span></div>
        <p>${words||esc(phrase.lyric_samples)}${phrase.lyric_samples_truncated?' …':''}</p>
        <small>${phrase.note_count} 音符 · ${phrase.seconds} 秒${gap?' · 句后留白 '+gap.seconds+' 秒 · 伴奏 '+gap.backing.pitched_attacks+' 次入音 / '+gap.backing.drum_hits+' 鼓击':''}</small>
        ${cue?'<small>'+esc(cue.question)+'</small>':''}
        ${phrase.harmony_listen_points?.length?'<small>和弦听点 · '+phrase.harmony_listen_points.map(point=>
          esc(point.symbol)+' / '+reviewBeatText(point.beat_range)).join(' · ')+'</small>':''}
        ${scoreListeningPoints(phrase,index,!!outputs[index])}
        <div class="score-review-actions">
          ${outputs[index]?'<button class="ghost" data-phrase-audition="'+index+'">对照</button>':''}
          ${!phrase.edge_clipped&&phrase.note_count>=4&&phrase.note_count<=16?'<button class="ghost" data-motif-index="'+index+'" title="在当前范围查找动机回归">动机</button>':''}
          <button class="ghost" data-review-index="${index}">定位</button>
        </div>
      </div>`;
    }).join('');
    const sections=detail.sections.map(section=>
      `<p><b>${esc(section.name)}</b> · ${reviewBeatText(section.beat_range)}<br>
      <small>每拍　人声 ${section.vocal_onsets_per_beat}　有音乐器 ${section.pitched_attacks_per_beat}　鼓 ${section.drum_hits_per_beat}</small></p>`).join('');
    const lyricRows=written.map((line,index)=>{
      const status={alignment_changed:'核对读音',missing_notes:'音符缺失'}[line.state];
      const time=line.beat_range?reviewBeatText(line.beat_range):'';
      const label=[line.track_name,time,status].filter(Boolean).map(esc).join(' · ');
      const text=esc(line.text).replace(/\r?\n/g,'<br>');
      return '<p>'+text+'<br><small>'+label+'</small>'+
        (time?' <button class="ghost" data-written-line="'+index+'">定位</button>':'')+'</p>';
    }).join('');
    body.innerHTML='<button id="sectionContrastOpen" class="ghost">段落对照</button><button id="rhythmInterplayOpen" class="ghost">鼓贝斯</button><p class="knowledge-scope">'+reviewBeatText(detail.beat_range)+' · 按谱面估算，听审判断实际重音与呼吸。</p>'+
      (outputs.some(Boolean)?'<section id="scorePhraseAudition" aria-label="同段对照">'+reviewAuditionControls('A 全混','B 人声轨',false,true)+
        '<div id="scorePhraseLevels" aria-label="分拍电平" hidden></div></section>':'')+
      '<section class="motif-review" id="scoreMotifReview" aria-label="动机回归结果" hidden></section>'+
      (lyricRows?'<section class="knowledge-card"><h3>原词</h3>'+lyricRows+
        (detail.lyric_lines_truncated?'<small>可框选段落细查</small>':'')+'</section>':'')+
      (phrases||'<p class="knowledge-scope">当前范围没有可复核的歌声音符。</p>')+
      (detail.phrases_truncated?'<p class="knowledge-scope">显示 '+detail.phrases.length+' / '+detail.total_phrases+' 句，可框选段落细查。</p>':'')+
      (sections?'<section class="knowledge-card"><h3>段落密度</h3>'+sections+'</section>':'')+
      (detail.audio_tracks_not_counted?'<p class="knowledge-scope">音频轨的演奏入场需试听核对。</p>':'');
    $('sectionContrastOpen').onclick=()=>void showSectionContrast(binding,detail.beat_range);
    $('rhythmInterplayOpen').onclick=()=>void showRhythmInterplay(binding,
      [detail.beat_range[0],Math.min(detail.beat_range[1],detail.beat_range[0]+16)]);
    for(const button of body.querySelectorAll('[data-written-line]')){
      button.onclick=()=>{const line=written[Number(button.dataset.writtenLine)];locateReviewPhrase(line,binding,line)};
    }
    for(const button of body.querySelectorAll('[data-review-index]')){
      button.onclick=()=>locateReviewPhrase(detail.phrases[Number(button.dataset.reviewIndex)],binding);
    }
    for(const button of body.querySelectorAll('[data-syllable-phrase]')){
      const phrase=detail.phrases[Number(button.dataset.syllablePhrase)];
      button.onclick=()=>locateReviewPhrase(phrase,binding,phrase.syllables[Number(button.dataset.syllableIndex)]);
    }
    for(const button of body.querySelectorAll('[data-listening-phrase]')){
      const phrase=detail.phrases[Number(button.dataset.listeningPhrase)];
      button.onclick=()=>locateReviewPhrase(phrase,binding,phrase.listening_points[Number(button.dataset.listeningIndex)]);
    }
    const phraseControls=$('scorePhraseAudition')?.firstElementChild;
    if(phraseControls){
      const prepare=wireReviewAudition(phraseControls,binding,({index,range},window)=>{
        const output=outputs[index],bounds=window||range;
        return {outputLabel:output.kind==='group'?'B 编组':'B 人声轨',
          args:{beat_ranges:[bounds,bounds],outputs:[null,[output]]}};
      });
      for(const button of body.querySelectorAll('[data-phrase-audition]')){
        const index=Number(button.dataset.phraseAudition);
        const target={index,range:detail.phrases[index].beat_range};
        button.onclick=()=>void prepare(target);
      }
      for(const button of body.querySelectorAll('[data-audition-point]')){
        const index=Number(button.dataset.auditionPhrase);
        const target={index,range:detail.phrases[index].listening_points[Number(button.dataset.auditionPoint)].audition_range};
        button.onclick=()=>void prepare(target);
      }
    }
    const motifPanel=$('scoreMotifReview');
    let requestVersion=0;
    for(const button of body.querySelectorAll('[data-motif-index]')){
      const search=async(wholeSong=false)=>{
        const version=++requestVersion;
        const phrase=detail.phrases[Number(button.dataset.motifIndex)];
        const [first_note_id,last_note_id]=phrase.boundary_note_ids;
        button.disabled=true;
        try{
          await releaseReviewPair();
          const result=await api('motif_returns',{...binding,anchor:{track_id:phrase.track_id,first_note_id,last_note_id},
            ...(!wholeSong&&range?{beat_range:detail.beat_range,track_ids:range.ids}:{})});
          if(!motifPanel.isConnected||version!==requestVersion||!knowledgeDialog.open)return;
          if(project?.id!==binding.project_id||project.revision!==binding.expected_revision)
            throw new Error('工程已更新，请重新复核');
          renderMotifReview(motifPanel,result,binding,!wholeSong&&range?()=>void search(true):null);
        }catch(error){if(version===requestVersion&&motifPanel.isConnected)toast(error.message,true)}
        finally{button.disabled=false}
      };
      button.onclick=()=>void search();
    }
    if(!knowledgeDialog.open)knowledgeDialog.showModal();
  }catch(error){toast(error.message,true)}
};
pollAgent();

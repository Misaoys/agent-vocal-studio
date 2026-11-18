'use strict';

const invokeNative=(op,args)=>window.desktopNative.invoke(op,args);
const editorFocus=()=>document.activeElement?.closest('input, textarea, select, [contenteditable=true]');
let closingDesktop=false;
const audioImportExtension=/\.(wav|mp3|flac)$/i;

async function importNativePaths(paths,target=null){
  if(!paths?.length)return;
  await editChain.catch(()=>{});
  let placement=target?.start;
  for(const path of paths){
    const file=path.split(/[\\/]/).at(-1);
    try{
      if(target&&project?.id!==target.project_id)throw new Error('工程已切换。请重新选择音频轨。');
      if(target&&!audioImportExtension.test(file))throw new Error('请选择 WAV、MP3 或 FLAC 文件');
      if(/\.zip$/i.test(file)){
        if(!project)throw new Error('先打开要接收分轨的工程');
        const projectId=project.id, count=project.tracks.length;
        const saved=await api('audio_import',{path,project_id:projectId,expected_revision:project.revision});
        if(project?.id!==projectId){await refreshList();return}
        trackId=saved.tracks[count].id;
        accept(saved);
        toast(`已导入 ${saved.tracks.length-count} 条对齐分轨`);
      }else if(audioImportExtension.test(file)){
        if(!project)throw new Error('先新建一个工程');
        const projectId=project.id;
        const saved=await api('audio_import',{
          path,project_id:projectId,expected_revision:project.revision,
          start:target?placement:cursorBeat,name:('伴奏 · '+file.replace(audioImportExtension,'')).slice(0,100),
          ...(target?{track_id:target.track_id}:{})
        });
        if(project?.id!==projectId){
          await refreshList();
          toast('音频已添加到原工程');
          return;
        }
        trackId=target?target.track_id:saved.tracks.at(-1).id;
        accept(saved);
        if(target){
          const track=tr(),clip=trackClips(track).find(c=>Math.abs(c.start-placement)<1e-8);
          if(clip){selectAudioClip(track,clip);placement=clipEndBeat(clip)}
        }
      }else{
        const imported=await importScore({path},file);
        if(!imported)continue;
        accept(imported.project);
        if(imported.warnings?.length)toast(imported.warnings.join(' '));
        else if(imported.archived_mix)toast('工程与归档混音已导入');
      }
      chooseGroup([]);
      await refreshList();
    }catch(error){toast(error.message,true)}
  }
}

async function closeDesktop(){
  if(closingDesktop||!window.desktopNative)return;
  closingDesktop=true;
  try{
    editorFocus()?.blur();
    let saved=true;
    try{await editChain}catch{saved=false}
    if(saved||await studioConfirm('修改未保存','仍要关闭当前工作区？','关闭'))await invokeNative('close');
  }finally{closingDesktop=false}
}

function jumpToStart(){
  cursorBeat=0;
  if(window.desktopNative||audio.readyState>0)audio.currentTime=0;
  $('rollViewport').scrollLeft=0;
  draw();
}

const commandDialog=document.createElement('dialog');
commandDialog.className='command-dialog pico';
commandDialog.setAttribute('aria-labelledby','commandTitle');
commandDialog.innerHTML='<div class="dialog-heading"><h2 id="commandTitle">命令</h2><button id="closeCommands" aria-label="关闭">'+icon('close')+'</button></div><input id="commandSearch" type="search" aria-label="搜索命令" placeholder="搜索命令" autocomplete="off"><div id="commandRows" role="list"></div>';
document.body.append(commandDialog);
wireDialog(commandDialog);
$('closeCommands').onclick=()=>commandDialog.close();

const outputDialog=document.createElement('dialog');
outputDialog.id='outputDialog';
outputDialog.className='arrange-dialog';
outputDialog.setAttribute('aria-labelledby','outputDialogTitle');
outputDialog.innerHTML=`
  <div class="dialog-heading"><h2 id="outputDialogTitle">音频输出</h2><button id="outputClose" aria-label="关闭">${icon('close')}</button></div>
  <label class="output-choice">设备<select id="outputChoice"></select></label>
  <output id="outputMessage" role="status"></output>
  <div class="arrange-dialog-footer"><button id="outputApply" class="primary">应用</button></div>`;
document.body.append(outputDialog);
wireDialog(outputDialog);
const outputButton=document.createElement('button');
outputButton.id='openOutput';
outputButton.type='button';
outputButton.title='音频输出';
outputButton.setAttribute('aria-label','音频输出');
outputButton.innerHTML=icon('speaker');
outputButton.hidden=!window.desktopNative;
$('timeDisplay').parentElement.append(outputButton);

async function openOutputSettings(){
  if(!window.desktopNative||outputDialog.open)return;
  const choice=$('outputChoice'),apply=$('outputApply');
  choice.replaceChildren();
  apply.disabled=true;
  $('outputMessage').textContent='';
  outputDialog.showModal();
  try{
    const state=await invokeNative('transport',{command:'output_devices'});
    if(!outputDialog.open)return;
    const current=state.output_preference;
    const fallback=state.default_output?'Windows 默认 · '+state.default_output:'Windows 默认';
    const seen=new Map();
    const choices=state.outputs.map(device=>{
      const ordinal=(seen.get(device.name)||0)+1;
      seen.set(device.name,ordinal);
      const duplicate=state.outputs.filter(item=>item.name===device.name).length>1;
      const label=device.name+(duplicate?` · 输出 ${ordinal}`:'');
      const rate=device.sample_rate?` · ${device.sample_rate/1000} kHz`:'';
      const option=new Option(label+rate,device.id);
      option.disabled=!device.selectable;
      return option;
    });
    if(current&&!choices.some(option=>option.value===current)){
      const unavailable=new Option('上次选用的设备 · 已断开',current);
      unavailable.disabled=true;
      choices.unshift(unavailable);
    }
    choice.append(new Option(fallback,''),...choices);
    choice.value=current??'';
    choice.onchange=()=>{apply.disabled=!!choice.selectedOptions[0]?.disabled};
    choice.onchange();
    $('outputMessage').textContent=state.active_output?'当前输出 · '+state.active_output:'';
  }catch(error){
    if(outputDialog.open)$('outputMessage').textContent=error.message;
  }
}

outputButton.onclick=()=>void openOutputSettings();
$('outputClose').onclick=()=>outputDialog.close();
$('outputApply').onclick=async()=>{
  const apply=$('outputApply'),choice=$('outputChoice'),close=$('outputClose');
  if(apply.disabled)return;
  apply.disabled=true;choice.disabled=true;close.disabled=true;
  try{
    await audio.selectOutput(choice.value||null);
    outputDialog.close();
  }catch(error){
    $('outputMessage').textContent=error.message;
  }finally{
    apply.disabled=false;choice.disabled=false;close.disabled=false;
  }
};
outputDialog.addEventListener('cancel',event=>{if($('outputClose').disabled)event.preventDefault()});

const click=id=>()=>$(id).click();
const commands=[
  ['new','新建工程','Ctrl+N','plus',click('newProject')],
  ['import','导入工程或伴奏','Ctrl+O','import',click('importBtn')],
  ['save','保存','Ctrl+S','check',async()=>{editorFocus()?.blur();await editChain;toast('已保存')}],
  ['bundle','另存工程包','Ctrl+Shift+S','folder',click('exportProject')],
  ['bundle_with_mix','另存带混音的工程包','','folder',saveProjectWithMix],
  ['play_archived_mix','试听归档混音','','play',playArchivedMix],
  ['export_audio','导出音频','Ctrl+E','export',click('exportWav')],
  ['export_midi','导出 MIDI','','midi',click('exportMidi')],
  ['undo','撤销','Ctrl+Z','undo',()=>history(false)],
  ['redo','重做','Ctrl+Shift+Z','redo',()=>history(true)],
  ['select_all','全选当前音轨','Ctrl+A','grid',()=>chooseGroup((tr()?.notes||[]).map(note=>note.id))],
  ['cut','剪切音符','Ctrl+X','scissors',()=>{if(copyPhrase())$('deleteNote').click()}],
  ['copy','复制音符','Ctrl+C','copy',()=>{if(copyPhrase())toast('已复制音符')}],
  ['paste','粘贴到编辑游标','Ctrl+V','copy',()=>pastePhrase(cursorBeat)],
  ['duplicate','复制乐句','Ctrl+D','copy',click('duplicateNotes')],
  ['delete','删除所选音符','Delete','close',click('deleteNote')],
  ['play','播放 / 暂停','Space','play',click('play')],
  ['stop','停止','Enter','stop',click('stop')],
  ['loop','切换选区循环','L','loop',click('loop')],
  ['record','录音','R','record',click('openRecording')],
  ['metronome','节拍器','M','metronome',click('metronomeToggle')],
  ['output_device','音频输出','','speaker',openOutputSettings],
  ['start','返回起点','Home','stop',jumpToStart],
  ['quantize','量化选区','Q','grid',click('quantizeNotes')],
  ['up','升半音','ArrowUp','up',()=>transpose(1)],
  ['down','降半音','ArrowDown','down',()=>transpose(-1)],
  ['octave_up','升八度','Shift+ArrowUp','up',()=>transpose(12)],
  ['octave_down','降八度','Shift+ArrowDown','down',()=>transpose(-12)],
  ['left','向前移动一格','Alt+ArrowLeft','left',()=>nudgeNotes(-1)],
  ['right','向后移动一格','Alt+ArrowRight','right',()=>nudgeNotes(1)],
  ['zoom_in','放大时间轴','Ctrl++','plus',click('zoomIn')],
  ['zoom_out','缩小时间轴','Ctrl+-','minus',click('zoomOut')],
  ['zoom_reset','还原缩放','Ctrl+0','grid',()=>{zoom=76;draw()}],
  ['arrangement','显示或收起编辑器','F2','grid',click('arrangeToggle')],
  ['arrange_transpose','多轨移调','','pitch',click('arrangeTranspose')],
  ['arrange_quantize','多轨量化','','grid',click('arrangeQuantize')],
  ['section_markers','编辑段落标记','','flag',click('editSections')],
  ['track_up','上移当前音轨','Alt+ArrowUp','up',()=>moveActiveTrack(-1)],
  ['track_down','下移当前音轨','Alt+ArrowDown','down',()=>moveActiveTrack(1)],
  ['track_duplicate','复制当前音轨','Ctrl+Shift+D','copy',()=>duplicateEditorTrack()],
  ['split_clip','切分音频片段','S','scissors',()=>{if(arrangementVisible)splitSelectedClip()}],
  ['review_split','预览切点与 A/B 试听','Shift+S','scissors',()=>{if(arrangementVisible)return reviewSplit()}],
  ['clip_slip','滑移片段内容','Alt+S','slip',openClipSlip],
  ['timeline','速度与拍号','','pitch',click('showTimeline')],
  ['pitch_join','起尾滑音 / 音符衔接','','pitch',click('pitchJoin')],
  ['pitch_vibrato','颤音包络','','pitch',click('pitchVibrato')],
  ['effects','混音效果器','F3','sliders',click('showEffects')],
  ['mixer','多轨混音器','F5','sliders',click('showMixer')],
  ['knowledge','创作知识','','book',click('showKnowledge')],
  ['palette','命令面板','Ctrl+K','grid',()=>openCommands(false)],
  ['shortcuts','快捷键','F1','grid',()=>openCommands(true)],
  ['about','关于','','wave',showStudioAbout]
];
const byId=new Map(commands.map(item=>[item[0],item]));
const byKey=new Map(commands.filter(item=>item[2]).map(item=>[item[2],item[0]]));

function nudgeNotes(direction){
  const notes=picked();
  if(!notes.length)return;
  const delta=direction*Number($('quantize').value);
  if(notes.some(note=>note.start+delta<0)){toast('所选音符已到工程起点');return}
  transformNotes(note=>({start:note.start+delta}));
}

let shortcutsOnly=false,activeCommand=0,shownCommands=[];
function highlightCommand(index){
  if(!shownCommands.length)return;
  activeCommand=(index+shownCommands.length)%shownCommands.length;
  const rows=$('commandRows').querySelectorAll('button');
  rows.forEach((row,i)=>row.classList.toggle('active',i===activeCommand));
  rows[activeCommand]?.scrollIntoView({block:'nearest'});
}
function drawCommands(){
  const query=$('commandSearch').value.trim().toLowerCase();
  shownCommands=commands.filter(row=>(!shortcutsOnly||row[2])&&(row[1]+' '+row[2]+' '+row[0]).toLowerCase().includes(query));
  const rows=shownCommands.map(([id,label,key,glyph],i)=>'<button type="button" data-command="'+id+'" role="listitem" class="'+(i===0?'active':'')+'">'+icon(glyph)+'<span>'+esc(label)+'</span>'+(key?'<kbd>'+esc(key.replace('ArrowUp','↑').replace('ArrowDown','↓').replace('ArrowLeft','←').replace('ArrowRight','→'))+'</kbd>':'')+'</button>');
  $('commandRows').innerHTML=rows.join('')||'<p>没有匹配的命令</p>';
  activeCommand=0;
}
function openCommands(onlyKeys){
  shortcutsOnly=onlyKeys;
  $('commandTitle').textContent=onlyKeys?'快捷键':'命令';
  $('commandSearch').value='';
  drawCommands();
  commandDialog.showModal();
  $('commandSearch').focus();
}
async function runPaletteCommand(id){
  if(!id)return;
  await new Promise(resolve=>{commandDialog.addEventListener('close',resolve,{once:true});commandDialog.close()});
  await execute(id);
}
$('commandSearch').oninput=drawCommands;
$('commandRows').onclick=event=>void runPaletteCommand(event.target.closest('[data-command]')?.dataset.command);
commandDialog.addEventListener('keydown',event=>{
  if(event.isComposing||event.keyCode===229)return;
  const delta={ArrowUp:-1,ArrowDown:1}[event.key];
  if(delta){event.preventDefault();highlightCommand(activeCommand+delta)}
  if(event.key==='Enter'&&event.target===$('commandSearch')){
    event.preventDefault();
    void runPaletteCommand(shownCommands[activeCommand]?.[0]);
  }
});

async function execute(id){
  const command=byId.get(id);
  if(!command||document.querySelector('dialog:modal'))return;
  try{
    if(workspaceFocus==='arrangement'){
      if(id==='paste'&&pasteClipGroup())return;
      const rangeMode=arrangeRangeTouchesNotes();
      if(!rangeMode&&clipGroupItems().length>1){
        if(id==='duplicate'){duplicateClipGroup();return}
        if(id==='copy'){copyClipGroup();return}
        if(id==='delete'){await quickGroupAction(groupRequest({kind:'remove'}));return}
        if(id==='left'||id==='right'){
          await quickGroupAction(groupRequest({kind:'move',delta:Number($('quantize').value)*(id==='left'?-1:1)}));return
        }
      }
      if(!rangeMode&&selectedAudioClip()){
        const clipCommands={delete:'remove',duplicate:'copy',copy:'copy'};
        if(clipCommands[id]){openClipDialog(clipCommands[id]);return}
        if(id==='cut'){toast('用 S 切分，或拖动片段边缘裁剪');return}
      }
      const scopeCommands={delete:'arrangeClear',duplicate:'arrangeCopy',copy:'arrangeCopy',cut:'arrangeMove',zoom_in:'arrangeZoomIn',zoom_out:'arrangeZoomOut'};
      if(scopeCommands[id]){$(scopeCommands[id]).click();return}
      if(id==='select_all'){
        const range={start:0,end:arrangementModel().limit,ids:project.tracks.map(t=>t.id)};
        updateArrangeSelection(range);selectClipsInRange(range);
        return;
      }
      if(id==='zoom_reset'){arrangeZoom=28;queueArrangementDraw();return}
      if(id==='start'){jumpToStart();arrangeViewport.scrollLeft=0;queueArrangementDraw();return}
      const semitones={up:1,down:-1,octave_up:12,octave_down:-12}[id];
      if(semitones&&!$('arrangeTranspose').disabled){openArrangeEdit('transpose',semitones);return}
      if(id==='quantize'){
        if($('arrangeQuantize').disabled)toast('请先框选包含音符的编排范围');
        else openArrangeEdit('quantize');
        return;
      }
      if(['paste','up','down','octave_up','octave_down','left','right'].includes(id)){
        toast('点击下方卷帘编辑音符');return;
      }
    }
    await command[4]();
  }catch(error){toast(error.message,true)}
}
window.EditorCommands=Object.freeze({execute});

document.addEventListener('keydown',event=>{
  if(event.defaultPrevented||event.isComposing||event.keyCode===229||document.querySelector('dialog:modal'))return;
  const ctrl=event.ctrlKey||event.metaKey;
  const key=event.code==='Space'?'Space':event.key.length===1?event.key.toUpperCase():event.key;
  const textFocused=editorFocus();
  const chord=(ctrl?'Ctrl+':'')+(event.altKey?'Alt+':'')+(event.shiftKey&&key!=='+'?'Shift+':'')+key;
  const id=chord==='Ctrl+Y'?'redo':chord==='Ctrl+='?'zoom_in':byKey.get(chord);
  if(!id||textFocused&&!['save','bundle','import','palette','shortcuts','mixer'].includes(id))return;
  if(['Enter','Space'].includes(key)&&event.target.closest('button')&&!ctrl)return;
  event.preventDefault();
  if(event.repeat&&!['up','down','octave_up','octave_down','left','right','zoom_in','zoom_out'].includes(id))return;
  void execute(id);
});

if(window.desktopNative){
  document.documentElement.dataset.desktop='true';
  window.desktopRequestClose=closeDesktop;
  window.desktopImportPaths=paths=>void importNativePaths(paths).catch(error=>toast(error.message,true));
  const open=async()=>{try{await importNativePaths(await studioChoose())}catch(error){toast(error.message,true)}};
  $('importBtn').onclick=$('emptyImport').onclick=open;
  let windowTitle=null;
  const refreshDesktop=refresh;
  refresh=function(external=false){
    refreshDesktop(external);
    if(!project||project.title===windowTitle)return;
    windowTitle=project.title;
    void invokeNative('title',{title:windowTitle}).catch(()=>{});
  };
  void invokeNative('ready').catch(error=>toast(error.message,true));
}

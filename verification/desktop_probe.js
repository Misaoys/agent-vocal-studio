void (async()=>{
  const report={};
  const check=(value,description)=>{if(!value)throw new Error(description)};
  async function until(predicate,message,timeout=12000){
    const end=performance.now()+timeout;
    while(performance.now()<end){if(predicate())return;await new Promise(resolve=>setTimeout(resolve,40))}
    throw new Error(message);
  }
  const key=(target,key,properties={})=>target.dispatchEvent(new KeyboardEvent('keydown',{key,bubbles:true,cancelable:true,...properties}));
  try{
    const host=await invokeNative('host_info');
    check(host.mode==='desktop'&&location.origin==='http://avs.studio'&&!token,'Desktop private origin did not load');
    check(host.chrome==='studio'&&$('studioClose')&&$('studioMenu'),'The custom Studio shell did not mount');
    key(document.body,'F4');
    check(!$('openAgentTask')&&!$('agentConversation')&&!$('agentBase')&&
      !commands.some(item=>item[0]==='agent_task'),'Built-in model controls were visible');
    report.external_agent_only=true;
    const faces=await Promise.all([document.fonts.load('500 12px "Studio Inter"','Agent'),document.fonts.load('500 12px "Studio Han"','工程あ')]);
    check(faces.every(items=>items.some(face=>face.status==='loaded')),'Bundled interface fonts did not load');
    report.custom_chrome_and_bundled_fonts=true;
    const readRgb=css=>css.match(/\d+/g).slice(0,3).map(Number);
    const neutral=channels=>Math.max(...channels)-Math.min(...channels)<=5;
    for(const selector of ['.topbar','.transport','.arrange-toolbar','.agent-activity','#play','#exportWav','.statusbar','#engineDot']){
      check(neutral(readRgb(getComputedStyle(document.querySelector(selector)).backgroundColor)),`${selector} gained a color outside the tracks`);
    }
    const pointer=document.querySelector('#agentCursor polygon');
    check(pointer&&neutral(readRgb(getComputedStyle(pointer).stroke))&&pointer.getAttribute('stroke-linejoin')==='round'&&!agentCursor.querySelector('span'),'Agent pointer is not the monochrome rounded triangle');
    report.neutral_chrome=true;
    const windowStatus=()=>invokeNative('window_control',{action:'state'});
    const originalWindow=await windowStatus();
    check(originalWindow.shadow,'The custom window did not enable its frame shadow');
    if(originalWindow.maximized){$('studioMaximize').click();await until(()=>document.documentElement.dataset.maximized==='false','The custom control did not restore the window')}
    const moveWindow=async(dx,dy)=>{
      const grip=$('studioDrag');
      for(const [name,x,y] of [['pointerdown',200,90],['pointermove',200+dx,90+dy],['pointerup',200+dx,90+dy]]){
        grip.dispatchEvent(new PointerEvent(name,{pointerId:83,button:0,bubbles:true,screenX:x,screenY:y}));
      }
      await new Promise(resolve=>setTimeout(resolve,120));
      return windowStatus();
    };
    const origin=await windowStatus();
    const shifted=await moveWindow(23,17);
    check(Math.abs(shifted.position[0]-origin.position[0]-Math.round(23*origin.scale))<=2,'The custom handle did not move the desktop window');
    const restored=await moveWindow(-23,-17);
    check(restored.position.every((position,i)=>Math.abs(position-origin.position[i])<=2),'Window drag did not restore the original placement');
    if(originalWindow.maximized){$('studioMaximize').click();await until(()=>document.documentElement.dataset.maximized==='true','The custom control did not maximize the window')}
    report.custom_drag_and_shadow=true;
    check((await api('studio_status')).version===host.version,'Desktop RPC version did not match host');
    report.native_window_and_private_origin=true;
    report.host_version=host.version;
    let fixture=await api('project_create',{title:'桌面交互验证 · 非成品歌曲',bpm:120});
    const pid=fixture.id,tid=fixture.tracks[0].id;
    fixture=await api('composition_design',{project_id:pid,expected_revision:fixture.revision,plan:{purpose:'technical_test',brief:'Desktop shortcut and variable time map engineering fixture.'}});
    fixture=await api('project_edit',{project_id:pid,expected_revision:fixture.revision,operations:[
      {op:'set_project',values:{tempo_map:[{beat:4,bpm:60,curve:'step'},{beat:8,bpm:120,curve:'linear'}],meter_map:[{beat:8,meter:[6,8]}]}},
      {op:'add_notes',track_id:tid,notes:[{start:1,duration:.5,pitch:60,lyric:'あ'},{start:4.5,duration:1.5,pitch:64,lyric:'い'},{start:7.5,duration:.5,pitch:67,lyric:'う'},{start:9.5,duration:.5,pitch:69,lyric:'え'}]}
    ]});
    await load(pid);
    const query=await api('timeline_query',{project_id:pid,beats:[4,6,8,9.5]});
    check(query.beat_positions.every(item=>Math.abs(beatSeconds(item.quarter_beat)-item.seconds)<1e-10),'Renderer and Rust disagree about timing');
    report.timeline_clock_parity=true;
    const sectionRevision=project.revision;
    $('editSections').click();
    check(sectionDialog.open,'Section marker editor did not open');
    $('sectionAdd').click();
    const sectionRow=$('sectionRows').firstElementChild;
    sectionRow.querySelector('[data-field=name]').value='副歌定位';
    sectionRow.querySelector('[data-field=start]').value='4';
    sectionRow.querySelector('[data-field=end]').value='6';
    $('sectionSave').click();
    await until(()=>project.revision>sectionRevision&&!sectionDialog.open,'Section marker did not save');
    check(project.section_markers?.[0]?.name==='副歌定位'&&project.format==='agent-vocal/23','Section marker was not persisted');
    const sectionView=await api('arrangement_query',{project_id:pid,expected_revision:project.revision});
    check(sectionView.sections[0].start_seconds===query.beat_positions[0].seconds&&
      sectionView.sections[0].end_seconds===query.beat_positions[1].seconds,'Section marker ignored local tempo');
    const markerBox=arrangeCanvas.getBoundingClientRect();
    arrangeCanvas.dispatchEvent(new PointerEvent('pointerdown',{button:0,clientX:markerBox.left+arrangeLabelWidth+4.5*arrangeZoom-arrangeViewport.scrollLeft,clientY:markerBox.top+29}));
    check(arrangeSelection?.start===4&&arrangeSelection.end===6&&cursorBeat===4,'Section marker click did not select and locate its range');
    await history(false);
    check(project.section_markers===undefined,'Undo did not restore the previous marker state');
    updateArrangeSelection(null);
    cursorBeat=0;
    report.desktop_section_markers=true;
    document.activeElement?.blur();
    $('rollViewport').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}));
    key(document.body,'a',{ctrlKey:true});
    check(picked().length===4,'Ctrl+A did not select notes');
    const initial=project.revision;
    key(document.body,'ArrowUp');
    await until(()=>project.revision>initial,'Transpose did not commit');
    check(tr().notes[0].pitch===61,'Wrong transpose result');
    key(document.body,'z',{ctrlKey:true});
    await until(()=>project?.id===pid&&tr()?.notes[0]?.pitch===60,'Undo did not restore the note in the open project');
    const afterUndo=project.revision;
    $('projectTitle').focus();
    key($('projectTitle'),'ArrowUp');
    await new Promise(resolve=>setTimeout(resolve,150));
    check(project.revision===afterUndo,'Typing focus caused a score edit');
    $('projectTitle').blur();
    report.shortcuts_and_text_focus=true;
    const beforeLyrics=structuredClone(project);
    chooseGroup(tr().notes.slice(0,2).map(note=>note.id));
    $('batchLyrics').click();
    await until(()=>$('lyricsDialog').open,'Lyrics selection did not open the dialog');
    $('lyricsText').value='キャッテ';
    $('lyricsText').dispatchEvent(new Event('input'));
    $('lyricsPreview').click();
    await until(()=>$('lyricsCount').textContent.includes('3 个读音 · 2 个选中音符'),'The lyrics preview did not reject a count mismatch');
    check($('applyLyrics').disabled&&project.revision===beforeLyrics.revision,'Invalid lyrics enabled commit');
    $('lyricsSource').value='夜';
    $('lyricsSource').dispatchEvent(new Event('input'));
    $('lyricsRead').click();
    await until(()=>$('lyricsText').value==='ヨ ル','The desktop did not extract and show a kanji reading');
    $('lyricsPreview').click();
    await until(()=>$('lyricsAlignment').children.length===2&&!$('applyLyrics').disabled,'Lyrics preview did not bind both notes');
    const savedBeforeLyrics=await api('project_read',{project_id:pid});
    check(savedBeforeLyrics.revision===beforeLyrics.revision,'Lyrics preview wrote to the project');
    $('applyLyrics').click();
    await until(()=>!$('lyricsDialog').open&&project.revision===beforeLyrics.revision+1,'Lyrics did not commit as one revision');
    check(tr().notes.every((note,i)=>JSON.stringify(note)===JSON.stringify(i<2?{...beforeLyrics.tracks[0].notes[i],lyric:['ヨ','ル'][i]}:beforeLyrics.tracks[0].notes[i])),
      'Changing lyrics changed timing, expression or notes outside the selection');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(beforeLyrics.tracks),'Undo did not restore the lyrics');
    report.desktop_lyrics={kanji_reading:true,scoped_preview:true,rejected_count_mismatch:true,untouched_others:true,one_undo:true};
    key(document.body,'k',{ctrlKey:true});
    check(commandDialog.open,'Ctrl+K did not open commands');
    $('commandSearch').value='速度';
    $('commandSearch').dispatchEvent(new Event('input',{bubbles:true}));
    key($('commandSearch'),'Enter');
    await until(()=>$('timelineDialog').open,'Palette did not dispatch timeline command');
    check(document.querySelector('.timeline-row[data-index="1"] [data-field=curve]').value==='linear','Timeline panel did not load saved slope');
    $('closeTimeline').click();
    await until(()=>!$('timelineDialog').open,'Timeline close animation did not finish');
    report.command_palette_and_timeline_panel=true;
    $('renderEngine').value='guide';
    cursorBeat=6;
    await render(true);
    await until(()=>!audio.paused&&audio.currentTime>=beatSeconds(6),'Desktop audio did not begin at mapped cursor');
    check(audio.currentTime<beatSeconds(6)+.5,'Desktop preview began at the wrong time');
    const transport=async()=>invokeNative('transport',{command:'status'});
    const playing=await transport();
    check(playing.backend==='WASAPI'&&playing.output_rate>0&&playing.output_channels>0,'Native audio output not opened');
    const converter=playing.output_rate===48000?'native_48khz':'band_limited_sinc';
    check(playing.sample_rate_conversion===converter,'Unexpected conversion mode for the actual WASAPI output rate');
    audio.currentTime=beatSeconds(5);
    await new Promise(resolve=>setTimeout(resolve,175));
    const metered=await transport();
    check(metered.callbacks>playing.callbacks&&metered.audio_frames>playing.audio_frames,'WASAPI did not request frames');
    check(metered.peak.every(peak=>peak>0&&peak<=1),'Native output level did not reflect the guide mix');
    report.desktop_wasapi_output={device:metered.device,rate:metered.output_rate,channels:metered.output_channels,
      converter,callback_delta:metered.callbacks-playing.callbacks,peak:metered.peak};
    const remote=args=>api('desktop_transport',args);
    const beforeDevices=await remote({action:'status'});
    const endpoints=await remote({action:'devices'});
    const deviceReadback=await remote({action:'status'});
    check(endpoints.backend==='WASAPI'&&endpoints.attached&&endpoints.active_output===metered.device&&
      beforeDevices.ticket===deviceReadback.ticket&&deviceReadback.playing,
      'MCP output inventory changed transport or did not list the active endpoint');
    const refusedOutput=await invokeNative('transport',{command:'output_device',device_id:null}).then(()=>null,error=>error.message);
    check(refusedOutput?.includes('停止播放')&&(await remote({action:'status'})).playing,
      'Changing output during playback did not preserve the active stream');
    async function denied(args,code){
      let failure;
      try{await remote(args)}catch(error){failure=String(error.message)}
      check(failure?.includes(code),`Expected ${code}, received ${failure}`);
    }
    check(playing.live_mix&&playing.monitor_revision===project.revision&&!rendered&&
      !(await api('render_jobs',{project_id:pid,expected_revision:project.revision,engine:'guide'})).jobs.length,
      'Desktop Play produced an offline render before starting the monitor');
    report.direct_monitor_without_export=true;
    await render(false);
    const request={action:'play',project_id:pid,expected_revision:project.revision,job_id:rendered.job_id,beat_range:[4.5,5],reason:'复听人声进句 · 验证'};
    const human=await remote({action:'status'});
    check(human.owner==='user'&&human.playing,'Human playback ownership not visible to the agent');
    await denied({...request,ticket:human.ticket},'USER_TRANSPORT_ACTIVE');
    audio.pause();
    const paused=await transport();
    await new Promise(resolve=>setTimeout(resolve,110));
    check(!(await transport()).playing&&(await transport()).audio_frames===paused.audio_frames,'Pause still consumed source frames');
    chooseGroup(tr().notes.slice(1,3).map(note=>note.id));
    $('loop').click();
    audio.currentTime=beatSeconds(9.5);
    await render(true);
    await until(()=>!audio.paused&&audio.currentTime>=beatSeconds(4.5),'Mapped loop did not start');
    check(audio.currentTime<beatSeconds(4.5)+.4,'Mapped loop start was wrong');
    const loopStart=await transport();
    const loopEndTime=beatSeconds(8);
    check(loopStart.loop_frames[0]===Math.round(beatSeconds(4.5)*48000)&&loopStart.loop_frames[1]===Math.round(loopEndTime*48000),'Loop points did not map to source frames');
    audio.currentTime=loopEndTime-.08;
    await new Promise(resolve=>setTimeout(resolve,210));
    const wrapped=await transport();
    check(wrapped.loop_wraps>loopStart.loop_wraps&&wrapped.seconds>=beatSeconds(4.5)&&wrapped.seconds<beatSeconds(4.5)+.4,'Native callback did not wrap to the selected interval');
    report.desktop_wasapi_frame_loop={boundaries:wrapped.loop_frames,wraps:wrapped.loop_wraps};
    $('stop').click();
    $('loop').click();
    check(audio.paused&&cursorBeat===0,'Stop did not reset playback and cursor');
    report.desktop_audio_seek_loop_stop=true;
    const idle=await remote({action:'status'});
    const initialSequence=(await api('agent_events')).sequence;
    const once=await remote({...request,ticket:idle.ticket});
    check(once.owner==='agent'&&once.stop_frame===Math.round(beatSeconds(5)*48000)&&once.loop_frames===null,'Agent one-shot did not arm its range');
    await until(()=>audio.agentRange?.[0]===once.preview_seconds[0]&&!audio.paused,'Remote playback did not update the desktop controls');
    await until(()=>audio.paused,'Agent one-shot did not finish');
    const ended=await remote({action:'status'});
    check(ended.ended&&Math.abs(ended.seconds-ended.stop_frame/48000)<1e-10&&ended.audio_frames>once.audio_frames,
      `Agent one-shot did not stop on its source frame (owner=${ended.owner}, ended=${ended.ended}, seconds=${ended.seconds}, frame=${ended.stop_frame})`);
    const playbackEvent=(await api('agent_events',{after:initialSequence})).events.find(item=>item.kind==='desktop_transport'&&item.reason===request.reason);
    check(playbackEvent,'Agent playback intent was not published');
    await until(()=>agentCursor.classList.contains('visible')&&['复听人声进句 · 验证','选段试听结束','已发起选段试听'].includes($('agentAction').textContent),'Agent playback did not move the drawn cursor and update the status');
    report.agent_once_focused_and_finished=ended.stop_frame;
    const looped=await remote({...request,repeat:true,ticket:ended.ticket});
    await until(()=>!audio.paused,'Agent loop did not enter playing state');
    await new Promise(resolve=>setTimeout(resolve,620));
    check((await remote({action:'status'})).loop_wraps>looped.loop_wraps,'Agent loop never wrapped');
    $('stop').click();
    await denied({action:'pause',ticket:looped.ticket},'STALE_TRANSPORT_TICKET');
    const takeover=await remote({action:'status'});
    check(takeover.owner==='user'&&!takeover.playing&&takeover.seconds===0,'Human stop did not take over the transport');
    await denied({action:'pause',ticket:takeover.ticket},'AGENT_PREVIEW_NOT_OWNER');
    report.agent_loop_user_takeover=true;
    const hardware=endpoints.outputs.find(device=>device.name===metered.device&&device.id!==metered.output_device_id&&device.selectable)
      ||endpoints.outputs.find(device=>device.id===metered.output_device_id&&device.selectable);
    check(hardware,'The active WASAPI output cannot be selected by its stable device ID');
    try{
      $('openOutput').click();
      await until(()=>outputDialog.open&&!$('outputApply').disabled,'The output selector did not load');
      check([...$('outputChoice').options].some(option=>option.value===hardware.id&&!option.disabled),'Available endpoint was absent from the desktop selector');
      $('outputChoice').value=hardware.id;
      $('outputApply').click();
      await until(()=>!outputDialog.open,'The output setting did not save');
      const configured=await remote({action:'status'});
      check(configured.output_preference===hardware.id,'MCP did not see the saved output preference');
      const unavailable=hardware.id+'-unavailable';
      const rejected=await invokeNative('transport',{command:'output_device',device_id:unavailable}).then(()=>null,error=>error.message);
      check(rejected?.includes('不可用')&&(await remote({action:'status'})).output_preference===hardware.id,
        'An invalid output choice replaced the valid one');
      await render(true);
      await until(()=>!audio.paused,'Playback did not open the selected WASAPI endpoint');
      await new Promise(resolve=>setTimeout(resolve,95));
      const selectedOutput=await transport();
      check(selectedOutput.output_device_id===hardware.id&&selectedOutput.output_rate===hardware.sample_rate&&
        selectedOutput.audio_frames>0,'The selected endpoint did not receive audio');
      $('stop').click();
      await until(()=>audio.paused,'Playback did not stop on the selected device');
      const defaults=await remote({action:'devices'});
      check(defaults.default_output_id===endpoints.default_output_id,'Studio changed Windows default audio output');
      report.desktop_output_selection={device_readback:true,preference_opened_stream:true,
        reject_unavailable:true,read_only_mcp:true,rate:hardware.sample_rate,available:endpoints.outputs.length,
        same_name_endpoints:endpoints.outputs.filter(item=>item.name===hardware.name).length,
        switched_between_same_name_endpoints:hardware.id!==metered.output_device_id};
    }finally{await audio.selectOutput(endpoints.output_preference)}
    const savedMix=rendered.job_id;
    rendered=null;
    await render(false);
    check(rendered?.job_id===savedMix,'The preview did not reuse an exact existing render');
    check((await api('render_jobs',{project_id:pid,expected_revision:project.revision,engine:'voicevox'})).jobs.every(item=>item.engine==='voicevox'),'Preview lookup crossed source engines');
    report.same_revision_render_reused=true;
    const expanded=await api('project_edit',{project_id:pid,expected_revision:project.revision,operations:[
      {op:'add_track',values:{name:'钢琴',instrument:{bank:0,program:0}}},
      {op:'set_project',values:{composition:{purpose:'technical_test',brief:'Range editing and desktop transport fixture.',sections:[{name:'开段',start:0,end:4,role:'定位测试'},{name:'发展',start:4,end:8,role:'多轨测试'}]}}}
    ]});
    const piano=expanded.tracks.at(-1).id;
    accept(await api('project_edit',{project_id:pid,expected_revision:expanded.revision,operations:[
      {op:'add_notes',track_id:piano,notes:[{start:4.5,duration:.5,pitch:48},{start:7.5,duration:.5,pitch:55}]}
    ]}));
    document.activeElement?.blur();
    check(arrangementPanel.getBoundingClientRect().height>100&&scorePanel.getBoundingClientRect().height>100,'Stacked editing workspace did not show both panes');
    check(arrangeViewport.contains($('tracks')),'Track controls were detached from the timeline');
    const initialHeight=scorePanel.getBoundingClientRect().height;
    key(divider,'ArrowUp');
    check(scorePanel.getBoundingClientRect().height>initialHeight,'Divider keyboard resize did not change pane height');
    key(document.body,'F2');
    await until(()=>scorePanel.hidden,'F2 did not collapse the editor');
    check(!arrangementPanel.hidden&&arrangeViewport.clientHeight>100,'Collapsing the editor hid the arrangement');
    key(document.body,'F2');
    await until(()=>!scorePanel.hidden&&arrangeCanvas.width>0&&arrangeCanvas.height>0,'F2 did not restore the simultaneous editing view');
    report.desktop_stacked_workspace_and_resize=true;
    check(arrangementModel().sections.map(s=>s.name).join(',')==='开段,发展','Arrangement section labels were lost');
    const rect=arrangeCanvas.getBoundingClientRect();
    const point=(beat,row)=>({clientX:rect.left+arrangeLabelWidth+beat*arrangeZoom,clientY:rect.top+arrangeHeader+row*arrangeRow+24,pointerId:17,button:0,bubbles:true});
    for(const [kind,beat,row] of [['pointerdown',4.05,0],['pointermove',7.95,1],['pointerup',7.95,1]])
      arrangeCanvas.dispatchEvent(new PointerEvent(kind,point(beat,row)));
    check(arrangeSelection?.start===4&&arrangeSelection?.end===8&&arrangeSelection.ids.length===2,'Multitrack box selection did not cover its tracks');
    await new Promise(resolve=>setTimeout(resolve,340));
    const published=await api('selection_resolve',{project_id:pid,from_ui:true});
    check(published.count===4,'Desktop selection did not publish all four exact notes');
    report.desktop_multitrack_scope_published=true;
    $('arrangeCopy').click();
    check(rangeDialog.open,'Copy range dialog did not open');
    $('arrangeTarget').value=12;
    $('arrangeTarget').dispatchEvent(new Event('input',{bubbles:true}));
    $('arrangePreview').click();
    await until(()=>$('arrangeApply').disabled===false,'Range preview failed: '+$('arrangePreviewText').textContent);
    check($('arrangePreviewText').textContent.includes('4 个音符'),'Range preview count was wrong');
    $('arrangeApply').click();
    await until(()=>!rangeDialog.open&&project.tracks.at(-1).notes.some(n=>n.start===12.5),'Copied range was not committed');
    check(project.tracks[0].notes.some(n=>n.start===15.5),'Copy did not retain relative timing on both tracks');
    report.desktop_arrangement_preview_commit=true;
    const copyRevision=project.revision;
    await history(false);
    check(project.revision>copyRevision&&!project.tracks[0].notes.some(n=>n.start>=12),'Arrangement Undo failed');
    report.desktop_arrangement_undo=true;
    await invokeNative('call',{name:'agent_focus',arguments:{project_id:pid,region:'arrangement',track_id:piano,beat_range:[4,8],reason:'核对发展段的钢琴音符'}});
    await until(()=>arrangeFocus?.id===piano&&$('agentCursor').classList.contains('visible'),'Agent cursor did not locate the arrangement range');
    check($('agentPhase').textContent==='准备'&&$('agentAction').textContent.includes('钢琴'),'Agent activity did not show its real next action');
    report.agent_action_status=true;
    report.desktop_arrangement_agent_focus=true;
    // Deterministic mono source for UI gestures. This is test audio, not a song.
    const clipBytes=new Uint8Array(44+48000*2);
    const wavView=new DataView(clipBytes.buffer);
    for(const [offset,tag] of [[0,'RIFF'],[8,'WAVE'],[12,'fmt '],[36,'data']]){
      clipBytes.set([...tag].map(c=>c.charCodeAt(0)),offset);
    }
    wavView.setUint32(4,clipBytes.length-8,true);
    wavView.setUint32(16,16,true);
    wavView.setUint16(20,1,true);wavView.setUint16(22,1,true);
    wavView.setUint32(24,48000,true);wavView.setUint32(28,96000,true);
    wavView.setUint16(32,2,true);wavView.setUint16(34,16,true);wavView.setUint32(40,96000,true);
    for(let i=0;i<48000;i++)wavView.setInt16(44+i*2,Math.round(6000*Math.cos(i/28)),true);
    let payload='';
    for(let i=0;i<clipBytes.length;i+=8192)payload+=String.fromCharCode(...clipBytes.subarray(i,i+8192));
    const withWav=await api('audio_import',{project_id:pid,expected_revision:project.revision,
      name:'音频测试片段',filename:'测试.wav',data_base64:btoa(payload),start:5});
    const backingId=withWav.tracks.at(-1).id;
    accept(withWav);
    {
      const track=project.tracks.at(-1);
      const seed={asset:'waveform-cache-probe.wav',offset:0,frames:4800,peaks:[[-.5,.5],[-.25,.25],[-.8,.8],[-.4,.4]]};
      const child={id:'waveform-cache-probe',asset:seed.asset,offset:1400,frames:1800};
      const exactKey=clipWaveformKey(child),seedKey=clipWaveformKey(seed);
      const call=api;
      let requests=0,requestedRevision=null;
      clipWaveforms.set(seedKey,seed);
      api=(name,args)=>{
        if(name!=='audio_waveform'||args.clip_id!==child.id)return call(name,args);
        requestedRevision=args.expected_revision;
        if(++requests===1)return Promise.reject(new Error('REVISION_CONFLICT'));
        return Promise.resolve({source_offset:child.offset,frames:child.frames,peaks:[[-.2,.3],[-.4,.6]]});
      };
      try{
        const preview=clipWaveform(track,child);
        check(preview?.peaks===seed.peaks&&preview.first===1&&preview.end===3&&preview.begin===1400,
          'The cut lost its visible covering bins while the exact waveform was pending');
        await until(()=>{
          const current=clipWaveform(track,child);
          return current?.peaks!==seed.peaks&&current?.stride===900;
        },'A failed waveform request permanently hid the trimmed waveform',3000);
        check(requests===2&&requestedRevision===project.revision,
          'The waveform request did not retry once against the visible revision');
        report.desktop_waveform_refresh={covering_preview:true,correct_window:true,retry_after_failure:true,revision_bound:true};
      }finally{
        api=call;
        clipWaveforms.delete(seedKey);
        clipWaveforms.delete(exactKey);
      }
    }
    arrangeViewport.scrollTo(0,0);
    drawArrangement();
    const audioRow=project.tracks.findIndex(t=>t.id===backingId);
    const clickClip=(kind,beat)=>{
      const r=arrangeCanvas.getBoundingClientRect();
      arrangeCanvas.dispatchEvent(new PointerEvent(kind,{clientX:r.left+arrangeLabelWidth+beat*arrangeZoom,
        clientY:r.top+arrangeHeader+audioRow*arrangeRow+28,pointerId:29,button:0,bubbles:true}));
    };
    clickClip('pointerdown',5.5);clickClip('pointerup',5.5);
    check(selectedAudioClip()?.clip.id==='legacy'&&cursorBeat===5.5,'Waveform selection did not set the split cursor');
    key(arrangeViewport,'s');
    await until(()=>project.tracks.at(-1).clips?.length===2,'S did not split audio: '+$('toast').textContent);
    const cut=project.tracks.at(-1).clips;
    check(cut[0].frames+cut[1].frames===48000,'Desktop split lost samples');
    check(cut[1].offset===cut[0].frames,'Desktop split did not retain source offset');
    report.desktop_clip_selection_and_split=true;
    const pivot=(cut[1].start+clipEndBeat(cut[1]))/2;
    clickClip('pointerdown',pivot);clickClip('pointermove',pivot+6);clickClip('pointerup',pivot+6);
    await until(()=>project.tracks.at(-1).clips?.find(clip=>clip.id===cut[1].id)?.start===11.5,'Drag did not move the selected clip');
    check(project.tracks.at(-1).clips[1].id===cut[1].id,'Drag changed the clip identity');
    report.desktop_clip_drag_move=true;
    $('clipSettings').click();
    check(clipDialog.open,'Clip inspector did not open');
    $('clipFadeIn').value=20;$('clipFadeOut').value=35;
    $('clipPreview').click();
    await until(()=>$('clipApply').disabled===false,'Fade preview did not succeed');
    $('clipApply').click();
    await until(()=>!clipDialog.open&&project.tracks.at(-1).clips[1].fade_in===960,'Fade edit was not saved');
    report.desktop_clip_fade_preview_commit=true;
    key(arrangeViewport,'z',{ctrlKey:true});
    await until(()=>project.tracks.at(-1).clips[1].fade_in===0,'Clip undo did not restore the fade');
    report.desktop_clip_undo=true;
    const beforeTrim=project.tracks.at(-1).clips[1];
    const leftMarker=beforeTrim.start+.025;
    clickClip('pointerdown',leftMarker);clickClip('pointermove',leftMarker+.35);clickClip('pointerup',leftMarker+.35);
    await until(()=>project.tracks.at(-1).clips[1].offset>beforeTrim.offset,'Drag on the clip edge did not trim');
    const trimmed=project.tracks.at(-1).clips[1];
    check(trimmed.id===beforeTrim.id&&trimmed.offset+trimmed.frames===beforeTrim.offset+beforeTrim.frames,'Left trim changed the source end or clip identity');
    report.desktop_clip_edge_trim=true;
    const trimmedTracks=structuredClone(project.tracks);
    const leftEdge=trimmed.start+.025;
    clickClip('pointerdown',leftEdge);
    check(clipDrag?.mode==='left','The shortened clip lost its left handle');
    clickClip('pointermove',leftEdge-.35);clickClip('pointerup',leftEdge-.35);
    await until(()=>project.tracks.at(-1).clips[1].offset<trimmed.offset,'Dragging left did not restore source audio');
    const extended=project.tracks.at(-1).clips[1];
    check(extended.id===trimmed.id&&extended.frames+extended.offset===trimmed.frames+trimmed.offset,
      'Left extension shifted the clip end or changed its ID');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(trimmedTracks),'Left extension did not undo');
    const marker=clipEndBeat(trimmed)-.025;
    clickClip('pointerdown',marker);clickClip('pointermove',marker-.24);clickClip('pointerup',marker-.24);
    await until(()=>project.tracks.at(-1).clips[1].frames<trimmed.frames,'Dragging the right edge inward did not crop the clip');
    const shorter=project.tracks.at(-1).clips[1];
    const shorterTrack=project.tracks.at(-1);
    await until(()=>{
      clipWaveform(shorterTrack,shorter);
      return clipWaveforms.get(clipWaveformKey(shorter))?.assetFrames===48000;
    },'The cropped waveform did not publish the source length');
    const shortRight=clipEndBeat(shorter)-.025;
    clickClip('pointerdown',shortRight);
    check(clipDrag?.mode==='right','The shortened clip lost its right handle');
    clickClip('pointermove',marker+.1);clickClip('pointerup',marker+.1);
    await until(()=>project.tracks.at(-1).clips[1].frames===trimmed.frames,'Dragging right did not restore the source tail');
    check(JSON.stringify(project.tracks)===JSON.stringify(trimmedTracks),'Right extension changed other clip properties');
    report.desktop_clip_edge_extend={both_handles:true,source_limit_from_waveform:true,one_undo:true};
    selectAudioClip(project.tracks.at(-1),trimmed);
    $('loop').click();
    const clipBounds=loopRange();
    check(Math.abs((clipBounds.end-clipBounds.start)*48000-trimmed.frames)<1e-5,'Selection in clip loop did not match sample duration');
    $('loop').click();
    report.desktop_clip_loop_range=true;
    const second=await api('audio_import',{project_id:pid,expected_revision:project.revision,
      name:'第二条伴奏',filename:'第二轨.wav',data_base64:btoa(payload),start:6});
    const secondId=second.tracks.at(-1).id;
    accept(second);
    check(new Set(project.tracks.map(trackColor)).size===project.tracks.length,'Different fixture tracks did not get distinct colors');
    report.only_tracks_use_color=true;
    arrangeViewport.scrollTo(0,0);
    drawArrangement();
    function gesture(kind,beat,row,properties={}){
      const r=arrangeCanvas.getBoundingClientRect();
      arrangeCanvas.dispatchEvent(new PointerEvent(kind,{clientX:r.left+arrangeLabelWidth+beat*arrangeZoom-arrangeViewport.scrollLeft,
        clientY:r.top+arrangeHeader+row*arrangeRow+26-arrangeViewport.scrollTop,pointerId:39,bubbles:true,button:0,...properties}));
    }
    const secondRow=project.tracks.findIndex(track=>track.id===secondId);
    gesture('pointerdown',5.1,audioRow,{shiftKey:true});
    gesture('pointermove',8.2,secondRow,{shiftKey:true});
    gesture('pointerup',8.2,secondRow,{shiftKey:true});
    check(clipGroupItems().length===2&&scorePanel.dataset.details==='audio'&&!$('inlineGroupFields').hidden,'A box across the audio tracks did not select a group');
    await until(()=>!scorePanel.hidden&&arrangementPanel.clientHeight>100,'Audio details replaced the arrangement');
    await new Promise(resolve=>setTimeout(resolve,330));
    const selectedClips=await api('audio_clips',{project_id:pid,from_ui:true});
    check(selectedClips.ui_clips.length===2&&new Set(selectedClips.ui_clips.map(ref=>ref.track_id)).size===2,'MCP could not read the two exact selected clips');
    const lanes=[...$('clipGroupList').querySelectorAll('.group-wave-row[data-track] canvas')];
    check(lanes.length===2&&lanes[0].clientWidth===lanes[1].clientWidth&&groupTimeline.start===5,
      'Grouped waveforms did not share one beat axis');
    await until(()=>lanes.every(canvas=>canvas.width>300),'The grouped waveforms did not paint');
    const geometry=lanes[1].getBoundingClientRect(),targetBeat=7;
    const beforeSeek=project.revision;
    lanes[1].dispatchEvent(new MouseEvent('click',{clientX:geometry.left+(targetBeat-groupTimeline.start)/
      (groupTimeline.end-groupTimeline.start)*geometry.width,bubbles:true}));
    check(Math.abs(cursorBeat-targetBeat)<.02&&project.revision===beforeSeek&&clipGroupItems().length===2,
      'A waveform seek changed the selection or project');
    const previousBeat=cursorBeat;
    key(lanes[1],'ArrowLeft');
    check(cursorBeat<previousBeat,'Grouped waveform did not support keyboard seeking');
    report.desktop_group_waveforms={aligned_tracks:true,seek_preserves_selection:true,keyboard_seek:true,mcp_selection_exact:true};
    const originalLanes=structuredClone(project.tracks);
    const reviewBeat=cursorBeat;
    key(lanes[1],'s');
    await until(()=>project.tracks.find(track=>track.id===secondId).clips?.length===2,'Multi-select S did not cut at the shared cursor');
    check(JSON.stringify(project.tracks.find(track=>track.id!==secondId&&track.id===originalLanes[audioRow].id))===
      JSON.stringify(originalLanes[audioRow]),'The cut changed a selected clip outside the cursor');
    await $('undo').onclick();
    check(JSON.stringify(project.tracks)===JSON.stringify(originalLanes),'One undo did not restore the group cut');
    gesture('pointerdown',5.1,audioRow,{shiftKey:true});
    gesture('pointermove',8.2,secondRow,{shiftKey:true});
    gesture('pointerup',8.2,secondRow,{shiftKey:true});
    report.desktop_group_split={cursor_only:true,one_undo:true};
    arrangementSeek(reviewBeat);
    const beforeReview=project.revision,priorSplitEngine=$('renderEngine').value;
    $('renderEngine').value='guide';
    key(arrangeViewport,'s',{shiftKey:true});
    await until(()=>splitDialog.open&&!$('applySplit').disabled,'Shift+S did not prepare a split preview');
    check(project.revision===beforeReview&&$('splitStatus').textContent.includes('第 '+Number(reviewBeat.toFixed(3))+' 拍'),
      'The split preview was not bound to the unsaved cursor position');
    splitDialog.querySelector('[data-listen]').click();
    await until(()=>splitAudition.binding(),'The split preview A/B did not render',25000);
    const splitBinding=splitAudition.binding();
    const cutAudio=await api('arrangement_audition',{project_id:pid,expected_revision:beforeReview,action:'status',
      audition_id:splitBinding.audition_id});
    const [listenFrom,listenTo]=cutAudio.result.beat_range.map(beat=>beatSeconds(beat));
    check(Math.abs(listenFrom-(beatSeconds(reviewBeat)-.3))<1e-6&&Math.abs(listenTo-(beatSeconds(reviewBeat)+.5))<1e-6,
      'The default split audition window missed the seam');
    await until(()=>audio.auditionId===splitBinding.audition_id&&audio.auditionVariant===1&&!audio.paused,
      'The split preview did not play B');
    splitDialog.querySelector('[data-side="A"]').click();
    await until(()=>audio.auditionVariant===0&&!audio.paused,'The split preview could not switch back to A');
    $('applySplit').click();
    await until(()=>!splitDialog.open&&project.revision===beforeReview+1,'The heard split was not committed');
    $('renderEngine').value=priorSplitEngine;
    await $('undo').onclick();
    check(JSON.stringify(project.tracks)===JSON.stringify(originalLanes),'Undo did not restore the auditioned cut');
    gesture('pointerdown',5.1,audioRow,{shiftKey:true});
    gesture('pointermove',8.2,secondRow,{shiftKey:true});
    gesture('pointerup',8.2,secondRow,{shiftKey:true});
    report.desktop_split_review={revision_bound:true,seam_window:true,native_ab:true,heard_preview_applied:true,one_undo:true};
    const beforeMove=clipGroupItems().map(item=>item.clip.start);
    gesture('pointerdown',5.25,audioRow);gesture('pointermove',9.25,audioRow);gesture('pointerup',9.25,audioRow);
    await until(()=>clipGroupItems().length===2&&clipGroupItems().every((item,i)=>Math.abs(item.clip.start-beforeMove[i]-4)<1e-8),'A drag did not move both clips in one transaction');
    const movedRevision=project.revision;
    key(arrangeViewport,'z',{ctrlKey:true});
    await until(()=>project.revision>movedRevision&&project.tracks.find(track=>track.id===secondId).audio?.start===6,'One undo did not restore both tracks');
    check(project.tracks.find(track=>track.id===backingId).clips[0].start===beforeMove[0],'Undo did not restore the first track');
    report.desktop_audio_group_selection_move_undo=true;
    await invokeNative('call',{name:'agent_focus',arguments:{project_id:pid,region:'arrangement',track_id:secondId,beat_range:[6,7],reason:'检查第二条伴奏'}});
    await until(()=>arrangeFocus?.id===secondId&&arrangeFocus.regions.length===1,'Agent did not highlight the requested audio track');
    check(trackId!==secondId,'Agent focus replaced the user active track');
    report.desktop_agent_focus_preserves_user_track=true;
    gesture('pointerdown',5.1,audioRow,{shiftKey:true});
    gesture('pointermove',8.2,secondRow,{shiftKey:true});
    gesture('pointerup',8.2,secondRow,{shiftKey:true});
    check(clipGroupItems().length===2&&!$('inlineGroupFields').hidden,'Group editing was not available after undo and reselect');
    await denied({...request,expected_revision:project.revision,ticket:(await remote({action:'status'})).ticket},'RENDER_REVISION_MISMATCH');
    const fixtureFolder=host.root+'\\verification\\native_file_fixture';
    async function inAppFolder(){
      await until(()=>filePanel.open,'The app file picker did not open');
      $('studioFilePath').value=fixtureFolder;
      $('studioFileGo').click();
      await until(()=>fileListing?.directory.endsWith('native_file_fixture'),'The app file picker did not navigate');
    }
    const beforeImport=project.revision;
    $('importBtn').click();
    await inAppFolder();
    const asset=[...$('studioFileEntries').querySelectorAll('[data-path]')].find(button=>button.dataset.path.endsWith('桌面文件对话框验证.wav'));
    check(asset,'Audio fixture was not listed in the app file dialog');
    asset.click();$('studioFileApply').click();
    await until(()=>!filePanel.open&&project.revision>beforeImport,'Import from the app picker was not saved');
    check(project.tracks.at(-1).audio?.seconds===.3,'The selected WAV was not imported as a new track');
    const fxTrack=project.tracks.find(track=>track.id===backingId);
    const fxRevision=project.revision;
    key(document.body,'F3');
    await until(()=>fxDialog.open,'F3 did not open the effects panel');
    $('studioFxScope').value=fxTrack.id;$('studioFxScope').dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxKind').value='eq';$('studioFxAdd').click();
    $('studioFxKind').value='compressor';$('studioFxAdd').click();
    const threshold=fxDialog.querySelector('[data-param=threshold_db]');
    threshold.value='-24';threshold.dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxScope').value='master';$('studioFxScope').dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxMasterEnabled').click();
    const limit=fxDialog.querySelector('[data-param=ceiling_db]');
    limit.value='-4';limit.dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxSave').click();
    await until(()=>!fxDialog.open&&project.revision>fxRevision,'Effects were not applied as a single edit: '+$('studioFxMessage').textContent);
    const applied=project.tracks.find(track=>track.id===backingId).effects;
    check(project.format==='agent-vocal/4'&&applied[0].kind==='eq'&&applied[1].threshold_db===-24&&project.master_limiter.ceiling_db===-4,'Effect settings were not persisted');
    report.desktop_effect_chain_and_master=true;
    const exported=exportFile('avpack');
    await inAppFolder();
    const localExport=`Studio_自绘文件选择_${Date.now()}.avpack`;
    $('studioFileName').value=localExport;
    await until(()=>$('studioFileApply').disabled===false,'Save destination never became ready');
    $('studioFileApply').click();
    await until(()=>!filePanel.open,'Save picker did not finish: '+$('studioFileStatus').textContent);
    await exported;
    const onDisk=await invokeNative('file_browse',{directory:fixtureFolder,extension:'avpack',query:localExport});
    check(onDisk.entries.length===1&&onDisk.entries[0].bytes>100,'The app save picker did not save a nonempty project bundle');
    report.custom_file_export_name=localExport;
    await render(false);
    check(rendered?.revision===project.revision,'Could not render the external MCP fixture');
    check(rendered.track_effects[0].track_id===backingId&&rendered.master_limiter.settings.ceiling_db===-4,'Desktop render did not process the saved effect chain');
    check(Number.isFinite(rendered.loudness.integrated_lufs)&&Number.isFinite(rendered.loudness.true_peak_dbtp)&&
      $('statusText').textContent.includes(' LUFS')&&$('statusText').textContent.includes(' dBTP'),
      'The completed export did not show its loudness and true peak');
    report.desktop_master_meter={reported_export_pcm:true,status_meters:true};
    report.agent_transport_fixture={project_id:pid,revision:project.revision,job_id:rendered.job_id};
    $('exportWav').click();
    await until(()=>waveExportDialog.open,'WAV export settings did not open');
    check($('waveExportSource').options.length===rendered.stems.length+2,'WAV export did not list the saved stems');
    $('waveExportSource').value='1';
    $('waveExportRate').value='44100';
    $('waveExportBits').value='16';
    $('waveExportSubmit').click();
    await inAppFolder();
    const waveName=`Studio_WAV_${Date.now()}.wav`;
    check($('studioFileName').value.endsWith('_44100Hz_16bit.wav'),'WAV format did not reach the native save picker');
    $('studioFileName').value=waveName;
    await until(()=>$('studioFileApply').disabled===false,'WAV destination never became available');
    $('studioFileApply').click();
    await until(()=>!waveExportDialog.open&&!filePanel.open,'WAV export did not finish');
    const waveFiles=await invokeNative('file_browse',{directory:fixtureFolder,extension:'wav',query:waveName});
    check(waveFiles.entries.length===1&&waveFiles.entries[0].bytes>44,'WAV export did not save a file');
    report.desktop_wave_delivery=true;
    $('exportWav').click();
    await until(()=>waveExportDialog.open,'Stem ZIP settings did not open');
    $('waveExportSource').value='all';
    $('waveExportSubmit').click();
    await inAppFolder();
    check($('studioFileName').value.endsWith('_44100Hz_16bit.zip'),'Stem ZIP format did not reach the native picker');
    const stemZipName=`Studio_Stems_${Date.now()}.zip`;
    $('studioFileName').value=stemZipName;
    await until(()=>$('studioFileApply').disabled===false,'Stem ZIP destination never became available');
    $('studioFileApply').click();
    await until(()=>!waveExportDialog.open&&!filePanel.open,'Stem ZIP save did not finish');
    const stemZips=await invokeNative('file_browse',{directory:fixtureFolder,extension:'zip',query:stemZipName});
    check(stemZips.entries.length===1&&stemZips.entries[0].bytes>44,'Stem ZIP did not save a file');
    report.desktop_stem_delivery=stemZips.entries[0].path;
    const previousRange=arrangeSelection;
    const chosenRange=[3.5,5.25];
    const selectedRange={start:chosenRange[0],end:chosenRange[1],ids:[project.tracks[0].id]};
    updateArrangeSelection(selectedRange);
    const exportBinding={project_id:pid,expected_revision:project.revision};
    let exportBox=null;
    for(let attempt=0;attempt<35;attempt++){
      const candidate=await api('arrangement_query',{...exportBinding,from_ui:true}).catch(()=>null);
      if(candidate?.beat_range?.every((beat,index)=>beat===chosenRange[index])){exportBox=candidate;break}
      await new Promise(resolve=>setTimeout(resolve,40));
    }
    check(exportBox?.box_id,'The export box did not publish its exact range');
    $('exportWav').click();
    await until(()=>waveExportDialog.open,'Range export settings did not open');
    check(!$('waveExportRangeLabel').hidden&&$('waveExportRange').options[1]?.text.includes('3.5–5.25'),'Range export did not show the captured selection');
    $('waveExportRange').value='box';
    $('waveExportRate').value='48000';
    $('waveExportBits').value='24';
    $('waveExportSubmit').click();
    await inAppFolder();
    check($('studioFileName').value.includes('_选区_48000Hz_24bit.wav'),'Range export did not reach the saver');
    const sliceName=`Studio_Range_${Date.now()}.wav`;
    $('studioFileName').value=sliceName;
    await until(()=>!$('studioFileApply').disabled,'Range export destination never became available');
    $('studioFileApply').click();
    await until(()=>!waveExportDialog.open&&!filePanel.open,'Range export save did not finish');
    const slices=await invokeNative('file_browse',{directory:fixtureFolder,extension:'wav',query:sliceName});
    check(slices.entries.length===1&&slices.entries[0].bytes>44,'Range export did not save its WAV');
    report.desktop_wave_selection={path:slices.entries[0].path,box_id:exportBox.box_id,beat_range:chosenRange};
    updateArrangeSelection(previousRange);
    const saveReference=EditorCommands.execute('bundle_with_mix');
    await inAppFolder();
    const archivedExport=`Studio_归档混音_${Date.now()}.avpack`;
    $('studioFileName').value=archivedExport;
    await until(()=>$('studioFileApply').disabled===false,'Archive destination never became ready');
    $('studioFileApply').click();
    await saveReference;
    const importedReference=await api('project_import',{path:fixtureFolder+'\\'+archivedExport});
    check(importedReference.archived_mix?.archived_mix&&importedReference.project.id!==pid,
      'Desktop bundle did not restore its completed mix');
    await load(importedReference.project.id);
    await EditorCommands.execute('play_archived_mix');
    await until(()=>!audio.paused&&$('statusText').textContent.startsWith('归档混音')&&playingArchivedMix,
      'The archived mix command did not play its WAV');
    const referenceTransport=await remote({action:'status'});
    check(referenceTransport.playing&&referenceTransport.project_id===project.id,
      'The archived WAV was not playing through native transport');
    $('stop').click();
    await load(pid);
    report.desktop_archived_mix=true;
    let mixFixture=await api('project_create',{title:'实时监听验证 · 非成品歌曲',bpm:120});
    mixFixture=await api('composition_design',{project_id:mixFixture.id,expected_revision:mixFixture.revision,plan:{purpose:'technical_test',brief:'Single-channel test tone for native monitor and source cache.'}});
    mixFixture=await api('audio_import',{project_id:mixFixture.id,expected_revision:mixFixture.revision,
      name:'监听测量信号',filename:'监听验证.wav',data_base64:btoa(payload),start:0});
    const sourceId=mixFixture.tracks.at(-1).id;
    mixFixture=await api('audio_import',{project_id:mixFixture.id,expected_revision:mixFixture.revision,
      name:'第二路监听信号',filename:'监听验证第二路.wav',data_base64:btoa(payload),start:0});
    const secondMixId=mixFixture.tracks.at(-1).id;
    mixFixture=await api('project_edit',{project_id:mixFixture.id,expected_revision:mixFixture.revision,
      operations:[{op:'update_track',track_id:secondMixId,values:{mute:true}}]});
    await load(mixFixture.id);
    const sourceTrack=project.tracks.find(track=>track.id===sourceId);
    selectAudioClip(sourceTrack,trackClips(sourceTrack)[0]);
    if(!looping)$('loop').click();
    cursorBeat=0;
    await render(true);
    await until(()=>!audio.paused,'Could not start the live mix fixture');
    await new Promise(resolve=>setTimeout(resolve,160));
    const dryLevel=await transport();
    check(dryLevel.peak[0]>.1&&dryLevel.monitor_track_ids.length===2&&!rendered,
      'Direct monitoring did not prepare the muted source or produced an offline mix');
    const tweakLiveEq=async()=>{
      document.activeElement?.blur();
      key(document.body,'F3');
      await until(()=>fxDialog.open,'Could not open the monitor controls');
      $('studioFxKind').value='eq';
      $('studioFxAdd').click();
      const hz=fxDialog.querySelector('[data-param=hz]');
      const gain=fxDialog.querySelector('[data-param=gain_db]');
      hz.value=273;hz.dispatchEvent(new Event('input',{bubbles:true}));
      gain.value=-18;gain.dispatchEvent(new Event('input',{bubbles:true}));
      await until(()=>audio.liveMix&&$('studioFxMessage').textContent.includes('实时监听'),
        'Could not switch to the live mix: '+$('studioFxMessage').textContent);
    };
    const mixRevision=project.revision;
    await tweakLiveEq();
    await new Promise(resolve=>setTimeout(resolve,570));
    const quiet=await transport();
    check(quiet.playing&&quiet.url===dryLevel.url&&quiet.audio_frames>dryLevel.audio_frames&&quiet.live_frames>0&&quiet.mix_changes>0,
      'Changing EQ restarted the source clock or did not process live device frames');
    const eqDelta=20*Math.log10(quiet.peak[0]/dryLevel.peak[0]);
    check(eqDelta < -15&&eqDelta > -21,`Live EQ changed the dry tone by ${eqDelta} dB`);
    check(project.revision===mixRevision&&!rendered,'Live draft changed the project or published a render');
    $('studioFxClose').click();
    await until(()=>!fxDialog.open,'Live effects did not close');
    await new Promise(resolve=>setTimeout(resolve,320));
    const restoredMix=await transport();
    check(restoredMix.playing&&restoredMix.peak[0]>.8*dryLevel.peak[0]&&restoredMix.mix_changes>quiet.mix_changes,'Closing draft did not restore saved effect settings');
    key(document.body,'F3');
    await until(()=>fxDialog.open,'Could not open the live master controls');
    $('studioFxScope').value='master';
    $('studioFxScope').dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxMasterEnabled').click();
    for(const [param,value] of [['input_db',12],['ceiling_db',-18]]){
      const field=fxDialog.querySelector('[data-param='+param+']');
      field.value=value;
      field.dispatchEvent(new Event('input',{bubbles:true}));
    }
    await new Promise(resolve=>setTimeout(resolve,530));
    const limitedMonitor=await transport();
    const masterPeak=20*Math.log10(limitedMonitor.peak[0]);
    check(limitedMonitor.playing&&limitedMonitor.mix_changes>restoredMix.mix_changes&&Math.abs(masterPeak+18)<.35,
      `Live master limiter reached ${masterPeak} dBFS`);
    $('studioFxClose').click();
    await until(()=>!fxDialog.open,'Live master controls did not close');
    await tweakLiveEq();
    $('studioFxSave').click();
    await until(()=>!fxDialog.open&&project.revision>mixRevision,'Could not apply monitored effects');
    const savedMonitor=await transport();
    check(savedMonitor.playing&&savedMonitor.live_mix&&savedMonitor.revision===project.revision&&savedMonitor.url===dryLevel.url,
      'Saving effects interrupted live playback or used stale project context');
    $('stop').click();
    await until(()=>audio.paused,'Could not stop live monitoring');
    await render(true);
    const resumedMonitor=await transport();
    check(resumedMonitor.playing&&resumedMonitor.live_mix&&resumedMonitor.url===dryLevel.url,'Play did not resume the saved live graph');
    document.activeElement?.blur();
    key(document.body,'F5');
    await until(()=>!mixerPanel.hidden&&channelViews.size===project.tracks.length,'F5 did not display every channel');
    check(arrangementPanel.clientHeight>100&&mixerPanel.clientHeight>80,'The mixer hid the arrangement');
    if(mixerPanel.clientHeight>=174)check(channelViews.get(sourceId).balance.getBoundingClientRect().bottom<=mixerPanel.getBoundingClientRect().bottom,
      'The pan control and readout did not fit in the mixer');
    const meter=(id,channel=0)=>audio.trackMeters[audio.monitorTrackIds.indexOf(id)]?.[channel]||0;
    await until(()=>meter(sourceId)>.015&&meter(secondMixId)===0,'Channel meter did not show the actual muted source state');
    const originalLevel=meter(sourceId);
    const fader=channelViews.get(sourceId).gain;
    const faderRevision=project.revision;
    fader.value=-12;
    fader.dispatchEvent(new Event('input',{bubbles:true}));
    await until(()=>meter(sourceId)>.15*originalLevel&&meter(sourceId)<.32*originalLevel,'Fader draft did not audibly change the channel');
    check(project.revision===faderRevision,'Dragging the mixer fader wrote project revisions');
    fader.dispatchEvent(new Event('change',{bubbles:true}));
    await until(()=>project.revision===faderRevision+1&&project.tracks.find(t=>t.id===sourceId).gain_db===-12,'Fader release did not commit once');
    const secondChannel=channelViews.get(secondMixId);
    const control=name=>secondChannel.strip.querySelector('[data-switch='+name+']');
    control('mute').click();
    await until(()=>meter(secondMixId)>.1,'Unmuting a prepared track did not bring it into the live mix');
    control('solo').click();
    await until(()=>meter(sourceId)<.001&&meter(secondMixId)>.1,'Solo did not silence the other channel');
    const panRevision=project.revision;
    secondChannel.pan.value=-1;
    secondChannel.pan.dispatchEvent(new Event('input',{bubbles:true}));
    await until(()=>meter(secondMixId,0)>.1&&meter(secondMixId,1)<.01,'Pan draft did not move the signal left');
    key(secondChannel.pan,'Escape');
    await until(()=>meter(secondMixId,0)>.1&&meter(secondMixId,1)>.1,'Cancelling pan did not restore both channels');
    check(project.revision===panRevision&&secondChannel.pan.valueAsNumber===0,'Cancelling pan wrote to the project');
    const mixerStatus=await transport();
    check(mixerStatus.playing&&mixerStatus.url===dryLevel.url&&mixerStatus.audio_frames>resumedMonitor.audio_frames,
      'Channel edits invalidated or restarted the source clock');
    check(secondChannel.meter[0].style.transform.startsWith('scaleY('),'The channel meter did not paint');
    const orderedIds=()=>project.tracks.map(track=>track.id).join(',');
    const stripOrder=()=>[...$('mixerStrips').querySelectorAll('[data-track]')].map(strip=>strip.dataset.track).join(',');
    const baselineOrder=orderedIds();
    const initialColors=new Map(project.tracks.map(track=>[track.id,trackColor(track)]));
    const activeTrackBefore=trackId;
    const clipBefore=selectedAudioClip()?.clip.id;
    const grip=tracksRoot.querySelector(`[data-track="${CSS.escape(sourceId)}"] [data-track-order]`);
    check(grip&&activeTrackBefore===sourceId,'The live track did not have a reorder handle or active editor binding');
    arrangeViewport.scrollTop=0;
    drawArrangement();
    const top=arrangeViewport.getBoundingClientRect().top+arrangeHeader+4;
    const cursor=grip.getBoundingClientRect();
    const pointerDrag=(node,type,y)=>node.dispatchEvent(new PointerEvent(type,{
      bubbles:true,cancelable:true,button:0,pointerId:26,clientX:cursor.left+6,clientY:y
    }));
    const beforeTrackDrag=project.revision;
    pointerDrag(grip,'pointerdown',cursor.top+7);
    pointerDrag(tracksRoot,'pointermove',top);
    check(!trackOrderMarker.hidden&&trackOrderMarker.isConnected&&project.revision===beforeTrackDrag,
      'Track dragging did not preview the insertion before saving');
    pointerDrag(tracksRoot,'pointerup',top);
    await until(()=>project.revision===beforeTrackDrag+1&&!trackOrderSaving,'Reordering a track did not save exactly once');
    check(project.tracks[0].id===sourceId&&orderedIds()===stripOrder()&&
      [...tracksRoot.querySelectorAll('[data-track]')].map(card=>card.dataset.track).join(',')===orderedIds(),
      'Arrangement and mixer did not share the new order');
    const reordered=orderedIds();
    check(trackId===activeTrackBefore&&selectedAudioClip()?.clip.id===clipBefore&&
      project.tracks.every(track=>initialColors.get(track.id)===trackColor(track))&&
      channelViews.get(secondMixId).strip===secondChannel.strip,'Track order changed selection, color, or channel identity');
    await until(()=>!audio.paused&&audio.liveMix,'Reordering invalidated live playback');
    const orderPlayback=await transport();
    check(orderPlayback.playing&&orderPlayback.url===mixerStatus.url&&orderPlayback.audio_frames>mixerStatus.audio_frames&&
      meter(secondMixId)>.1&&meter(sourceId)<.001,'Track reorder did not retain the live clock and meter identities');
    key(arrangeViewport,'ArrowDown',{altKey:true});
    await until(()=>orderedIds()===baselineOrder&&!trackOrderSaving,'Alt+Down did not move the active track down');
    key(arrangeViewport,'z',{ctrlKey:true});
    await until(()=>orderedIds()===reordered,'Undo did not restore the preceding track order');
    key(arrangeViewport,'ArrowDown',{altKey:true});
    await until(()=>orderedIds()===baselineOrder&&!trackOrderSaving,'Restoring the monitor order failed');
    report.desktop_track_order={drag_preview_and_commit:true,keys_and_undo:true,
      stable_color_selection_and_channels:true,live_clock_continued:true};
    report.desktop_mixer={visible_with_arrangement:true,muted_source_prepared:true,muted_source_absent_from_export:true,
      fader_draft_in_audio:true,fader_single_commit:true,unmute_and_solo_keep_playing:true,pan_and_cancel:true,
      device_frames_during_mix:mixerStatus.audio_frames-resumedMonitor.audio_frames};
    updateArrangeSelection(null);
    $('showGainAutomation').click();
    await until(()=>gainDialog.open&&gainDraft,'The volume curve did not open');
    $('gainTrack').value=secondMixId;
    await $('gainTrack').onchange();
    const gainBefore=project.revision,gainPeak=meter(secondMixId);
    $('gainAdd').click();
    $('gainPointDb').value=-12;
    $('gainPointDb').dispatchEvent(new Event('change'));
    check(project.revision===gainBefore&&gainSvg.querySelectorAll('circle').length===1,'Editing a draft wrote to the project');
    $('gainApply').click();
    await until(()=>!gainDialog.open&&project.revision===gainBefore+1,'The curve did not save once');
    await until(()=>meter(secondMixId)>.18*gainPeak&&meter(secondMixId)<.29*gainPeak,'The saved curve did not reach the WASAPI mixer');
    const curveMonitor=await transport();
    check(curveMonitor.playing&&curveMonitor.live_mix&&curveMonitor.url===mixerStatus.url&&
      curveMonitor.audio_frames>mixerStatus.audio_frames&&curveMonitor.mix_changes>mixerStatus.mix_changes,'Automation restarted playback');
    report.desktop_automation={graph_and_one_commit:true,live_gain_db:20*Math.log10(meter(secondMixId)/gainPeak),playback_continued:true};
    $('showMixer').click();
    await until(()=>mixerPanel.hidden,'Could not restore the editor after mixing');
    await edit([{op:'set_project',values:{tail_seconds:.5}}]);
    await until(()=>audio.paused&&!audio.liveMix,'Editing the source duration did not invalidate the live graph');
    if(looping)$('loop').click();
    report.desktop_live_mix={measured_eq_db:eqDelta,master_peak_dbfs:masterPeak,advanced_device_frames:quiet.audio_frames-dryLevel.audio_frames,
      callback_delta:quiet.callbacks-dryLevel.callbacks,contended_callback_delta:quiet.contended_callbacks-dryLevel.contended_callbacks,
      draft_reverted:true,save_kept_playing:true,resume_reused_graph:true,
      source_edit_invalidated:true};
    await edit([sourceId,secondMixId].map(track_id=>({op:'update_track',track_id,values:{mute:true,solo:false}})));
    await render(true);
    await until(()=>!audio.paused&&audio.liveMix,'Starting transport from an all-muted project failed');
    const mutedMonitor=await transport();
    check(!rendered&&mutedMonitor.peak.every(peak=>peak===0)&&mutedMonitor.monitor_track_ids.length===2,
      'Silent monitoring produced an offline mix, audible output, or discarded prepared sources');
    $('stop').click();
    report.desktop_mixer.all_muted_transport_can_start=true;
    const cloneTrackCount=project.tracks.length;
    const cloneSource=project.tracks.find(track=>track.id===sourceId);
    check(cloneSource.audio,'The duplicate test needs legacy audio');
    selectAudioClip(cloneSource,null);
    key(arrangeViewport,'d',{ctrlKey:true,shiftKey:true});
    await until(()=>project.tracks.length===cloneTrackCount+1&&!duplicatingTrack,'Ctrl+Shift+D did not duplicate the selected track');
    const clone=tr();
    check(clone.id!==sourceId&&clone.mute&&!clone.solo&&clone.name.endsWith('副本')&&
      clone.clips[0].id!=='legacy'&&clone.clips[0].asset===cloneSource.audio.asset&&clone.clips[0].start===cloneSource.audio.start,
      'The audio track duplicate did not become an independent muted copy of the same material');
    await history(false);
    check(project.tracks.length===cloneTrackCount,'Undo did not remove the track copy');
    tracksRoot.querySelector(`[data-duplicate-track="${CSS.escape(secondMixId)}"]`).click();
    await until(()=>project.tracks.length===cloneTrackCount+1&&!duplicatingTrack,'The track header copy button did not commit');
    check(project.tracks[project.tracks.findIndex(track=>track.id===secondMixId)+1].id===trackId,
      'The header copy action targeted the wrong track');
    await history(false);
    report.desktop_track_duplicate={header_and_keyboard:true,legacy_asset_and_fresh_clip:true,one_undo:true};
    let tuning=await api('project_create',{title:'桌面滑音技术验证 · 非成品歌曲',bpm:120});
    const tune=tuning.tracks[0].id;
    tuning=await api('composition_design',{project_id:tuning.id,expected_revision:tuning.revision,
      plan:{purpose:'technical_test',brief:'Scope and compare the expression at a touching vocal boundary.'}});
    tuning=await api('project_edit',{project_id:tuning.id,expected_revision:tuning.revision,operations:[
      {op:'set_project',values:{tempo_map:[{beat:4,bpm:100,curve:'linear'}]}},
      {op:'add_notes',track_id:tune,notes:[
        {start:0,duration:2,pitch:57,lyric:'ア',pitch_points:[[0,8],[1,5]]},
        {start:2,duration:2,pitch:64,lyric:'ア',pitch_points:[[1,-6],[2,4]]},
        {start:5,duration:1,pitch:62,lyric:'イ'}
      ]}
    ]});
    await load(tuning.id);
    workspaceFocus='editor';
    const [firstNote,secondNote,thirdNote]=tr().notes;
    chooseGroup([firstNote.id,thirdNote.id]);
    await $('pitchJoin').onclick();
    check(!$('pitchJoinDialog').open,'A gap should keep the pitch join dialog closed');
    chooseGroup([firstNote.id,secondNote.id]);
    $('pitchJoin').click();
    await until(()=>$('pitchJoinDialog').open&&!$('applyPitchJoin').disabled,'A touching pair did not get a pitch preview');
    check($('pitchJoinOriginal').getAttribute('points')!==$('pitchJoinDraft').getAttribute('points'),'The slide was not drawn');
    const beforeSlide=await api('project_read',{project_id:tuning.id});
    check(JSON.stringify(beforeSlide)===JSON.stringify(tuning),'Preview wrote the pitch curves');
    $('pitchJoinLead').value=140;
    $('pitchJoinLead').dispatchEvent(new Event('input'));
    check($('applyPitchJoin').disabled,'Changing slide duration kept an old preview active');
    $('previewPitchJoin').click();
    await until(()=>!$('applyPitchJoin').disabled,'The new duration did not preview');
    $('applyPitchJoin').click();
    await until(()=>!$('pitchJoinDialog').open&&project.revision===tuning.revision+1,'The slide did not save in one revision');
    const [voiceA,voiceB,unchanged]=tr().notes;
    check(Math.abs(voiceA.pitch*100+voiceA.pitch_points.at(-1)[1]-voiceB.pitch*100-voiceB.pitch_points[0][1])<1e-8&&
      JSON.stringify(unchanged)===JSON.stringify(thirdNote)&&voiceA.duration===firstNote.duration&&voiceB.lyric===secondNote.lyric,
      'Saved pitch curves were disconnected or moved unrelated material');
    check(pitchExpressionRange(voiceA)>200&&voiceA.pitch_points.every(point=>Math.abs(point[1])<=pitchExpressionRange(voiceA)),
      'Editing scale would clip the wide glide');
    const beforeRetouch=project.revision;
    selected=voiceA.id;mode='pitch';
    const expressionBounds=expr.getBoundingClientRect();
    setExpression({clientX:expressionBounds.left+beatX(voiceA.start+.75),clientY:expressionBounds.top+16});
    expr.onpointerup();
    await until(()=>project.revision===beforeRetouch+1,'Retouching the pitch curve did not save');
    check(tr().notes[0].pitch_points.some(point=>point[0]===.75&&point[1]>200)&&
      JSON.stringify(tr().notes[1])===JSON.stringify(voiceB),'Retouching one note also changed its neighbor');
    await history(false);
    await history(false);
    check(project.format===tuning.format&&JSON.stringify(project.tracks)===JSON.stringify(tuning.tracks),'Undo did not restore the original curves');
    report.desktop_pitch_join={charted_before_after:true,preview_no_write:true,one_undo:true,disjoint_pair_rejected:true,
      matching_boundary:true,unselected_note_preserved:true,wide_glide_scale:true,retouch_stays_on_one_note:true};
    chooseGroup([thirdNote.id]);
    const beforeEdge=project.revision;
    const priorPitchEngine=$('renderEngine').value;
    $('renderEngine').value='voicevox';
    await render(false);
    const pitchBaseJob=rendered.job_id;
    $('pitchJoin').click();
    await until(()=>$('pitchJoinDialog').open&&!$('applyPitchJoin').disabled,'A single note did not get an onset preview');
    check(!$('pitchEdgeFields').hidden&&$('pitchJoinFields').hidden&&
      $('pitchJoinOriginal').getAttribute('points')!==$('pitchJoinDraft').getAttribute('points'),
      'Single-note controls or preview curves were missing');
    $('pitchAuditionStart').click();
    await until(()=>pitchAudio?.status==='complete'&&audio.auditionId===pitchAudio.id&&audio.auditionVariant===1&&!audio.paused,
      'The original and onset draft were not prepared for native listening',25000);
    const onsetPair=pitchAudio.id;
    const onsetJob=await api('pitch_audition_status',{audition_id:onsetPair});
    const onsetProof=await invokeNative('verification_audition',{audition_id:onsetPair,reference_job_id:pitchBaseJob});
    check(onsetJob.result.source_inputs.join(',')==='saved_mix,rendered'&&onsetProof.matches_saved_mix&&onsetProof.distinct_pcm&&
      project.revision===beforeEdge,'Slide preview did not reuse A, render an audible B, or preserve the score');
    $('pitchAuditionA').click();
    await until(()=>audio.auditionId===onsetPair&&audio.auditionVariant===0&&!audio.paused,'A did not switch within the pitch comparison');
    $('pitchEdgeSide').value='end';
    $('pitchEdgeSide').dispatchEvent(new Event('input'));
    $('pitchEdgeOffset').value=-150;
    $('pitchEdgeOffset').dispatchEvent(new Event('input'));
    check($('applyPitchJoin').disabled&&project.revision===beforeEdge,'Changing the edge kept stale approval');
    await until(()=>!audio.auditionId,'Changing the slide left an outdated pair playing');
    $('previewPitchJoin').click();
    await until(()=>!$('applyPitchJoin').disabled,'The fall at the end did not preview');
    $('pitchAuditionStart').click();
    await until(()=>pitchAudio?.id!==onsetPair&&pitchAudio?.status==='complete'&&audio.auditionId===pitchAudio.id&&
      audio.auditionVariant===1&&!audio.paused,'Updating the pitch draft did not start a fresh comparison',25000);
    const tailPair=pitchAudio.id;
    const tailStatus=await api('pitch_audition_status',{audition_id:tailPair});
    const tailAudio=await invokeNative('verification_audition',{audition_id:tailPair,hash_audio:true});
    check(tailAudio.pcm_hashes[0]!==tailAudio.pcm_hashes[1],'The preview revisions sounded identical at their affected edge');
    const pitchRenders=(await api('render_jobs',{project_id:project.id})).jobs;
    check(pitchRenders.length===1&&pitchRenders[0].job_id===pitchBaseJob,
      'Temporary pitch comparisons published renders');
    $('applyPitchJoin').click();
    await until(()=>!$('pitchJoinDialog').open&&project.revision===beforeEdge+1,'The edge did not commit in one revision');
    check(!audio.auditionId&&!pitchAudio,'Applying the pitch draft did not stop its comparison');
    let pitchReleased=false;
    try{await invokeNative('verification_audition',{audition_id:tailPair})}catch{pitchReleased=true}
    check(pitchReleased,'The temporary slide audio was kept after commit');
    await render(false);
    check(rendered.reused_preview_voice_track_ids.join(',')===tune&&rendered.revision===project.revision,
      'Export did not reuse the exact auditioned dry voice');
    const approvedScope={project_id:project.id,expected_revision:project.revision};
    const approvedPair=await api('range_audition',{action:'prepare',...approvedScope,job_id:rendered.job_id,
      beat_ranges:[tailStatus.result.beat_range,[0,.25]],level_match:false});
    try{
      const approvedAudio=await invokeNative('verification_audition',{audition_id:approvedPair.id,hash_audio:true});
      check(approvedAudio.pcm_hashes[0]===tailAudio.pcm_hashes[1],
        'Approved export changed the already auditioned PCM');
    }finally{await api('range_audition',{action:'release',...approvedScope,audition_id:approvedPair.id})}
    $('renderEngine').value=priorPitchEngine;
    check(tr().notes[2].pitch_points.at(-1)[1]===-150&&tr().notes[2].duration===thirdNote.duration&&
      JSON.stringify(tr().notes.slice(0,2))===JSON.stringify([firstNote,secondNote]),
      'The fall changed timing, a neighbor or the wrong end of the note');
    await history(false);
    check(JSON.stringify(tr().notes[2])===JSON.stringify(thirdNote),'Undo did not restore the original note');
    report.desktop_pitch_edge={onset_preview_and_tail_commit:true,scope_and_undo:true,
      native_ab_same_revision:true,audio_followed_parameters_and_released:true,approved_preview_reused_and_pcm_exact:true};
    const beforeVibrato=project;
    chooseGroup([secondNote.id]);
    $('renderEngine').value='voicevox';
    $('pitchVibrato').click();
    await until(()=>$('pitchJoinDialog').open&&!$('pitchVibratoFields').hidden,'The note vibrato controls did not open');
    check($('pitchLocalPreview').checked&&$('applyPitchJoin').disabled,'Vibrato did not default to read-only dry preview');
    for(const [id,value] of Object.entries({Depth:54,Rate:5.4,Start:22,End:86,FadeIn:90,FadeOut:120}))
      $('vibratoShape'+id).value=value;
    $('previewPitchJoin').click();
    await until(()=>!$('applyPitchJoin').disabled,'The vibrato envelope did not preview');
    check($('pitchJoinOriginal').getAttribute('points')!==$('pitchJoinDraft').getAttribute('points')&&
      Number($('pitchVibratoWindow').getAttribute('width'))>0&&
      JSON.stringify(await api('project_read',{project_id:project.id}))===JSON.stringify(beforeVibrato),
      'Envelope plot leaked into the saved score or missed the preview region');
    $('pitchAuditionStart').click();
    await until(()=>pitchAudio?.status==='complete'&&audio.auditionId===pitchAudio.id&&audio.auditionVariant===1&&!audio.paused,
      'Vibrato did not render as a local A/B pair',25000);
    const vibratoPair=pitchAudio.id;
    const vibratoStatus=await api('pitch_audition_status',{audition_id:vibratoPair});
    const vibratoAudio=await invokeNative('verification_audition',{audition_id:vibratoPair,hash_audio:true});
    check(vibratoStatus.result.source_inputs.join(',')==='local_voice,local_voice'&&
      vibratoStatus.result.gain_db.every(gain=>gain===0)&&vibratoAudio.pcm_hashes[0]!==vibratoAudio.pcm_hashes[1],
      'The two vibrato sides were normalized, not local, or PCM-identical');
    $('pitchAuditionA').click();
    await until(()=>audio.auditionId===vibratoPair&&audio.auditionVariant===0&&!audio.paused,'Dry A/B did not switch inside the pair');
    $('applyPitchJoin').click();
    await until(()=>!$('pitchJoinDialog').open&&project.revision===beforeVibrato.revision+1,'Vibrato confirmation did not commit');
    const vibratoNotes=tr().notes;
    check(project.format==='agent-vocal/18'&&vibratoNotes[1].vibrato.end===.86&&vibratoNotes[1].vibrato.fade_out_ms===120&&
      JSON.stringify(vibratoNotes)===JSON.stringify([firstNote,{...secondNote,vibrato:vibratoNotes[1].vibrato},thirdNote])&&
      !audio.auditionId,'Vibrato changed neighboring notes, pitch points or left A/B playing');
    const quickChange=project.revision;
    selected=secondNote.id;inspect();
    $('vibratoRate').value=5.8;
    $('vibratoRate').dispatchEvent(new Event('change'));
    await until(()=>project.revision===quickChange+1,'The inspector rate did not save');
    check(JSON.stringify(tr().notes[1].vibrato)===JSON.stringify({...vibratoNotes[1].vibrato,rate:5.8}),
      'Quick vibrato controls removed the fade or range');
    await history(false);
    check(JSON.stringify(tr().notes)===JSON.stringify(vibratoNotes),'The quick edit did not undo');
    await history(false);
    check(project.format===beforeVibrato.format&&JSON.stringify(project.tracks)===JSON.stringify(beforeVibrato.tracks),
      'The vibrato envelope did not undo in a single step');
    report.desktop_vibrato={plotted_range:true,read_only_dry_ab:true,unchanged_neighbor_and_pitch_curve:true,
      inspector_preserves_envelope:true,one_undo:true};
    $('renderEngine').value=priorPitchEngine;
    let seamScore=await api('project_create',{title:'桌面交叉淡化技术验证 · 非成品歌曲',bpm:120});
    seamScore=await api('composition_design',{project_id:seamScore.id,expected_revision:seamScore.revision,
      plan:{purpose:'technical_test',brief:'Verify an in-place audio transition between two source windows.'}});
    seamScore=await api('audio_import',{project_id:seamScore.id,expected_revision:seamScore.revision,
      filename:'接缝测试.wav',name:'接缝测试',data_base64:btoa(payload),start:0});
    const seamLane=seamScore.tracks.at(-1).id;
    const cutAt={project_id:seamScore.id,expected_revision:seamScore.revision,track_id:seamLane,clip_id:'legacy',action:{kind:'split',at:1}};
    const seamPreview=await api('audio_clip_edit',cutAt);
    seamScore=(await api('audio_clip_edit',{...cutAt,preview_id:seamPreview.preview_id})).project;
    await load(seamScore.id);
    const markSeam=()=>{
      const track=project.tracks.find(item=>item.id===seamLane);
      setClipGroup(track.clips.map(clip=>({track,clip})));
      workspaceFocus='arrangement';
    };
    markSeam();
    check(!$('groupCrossfade').hidden,'The two clips on the same track did not offer crossfade');
    $('groupCrossfade').click();
    await until(()=>crossfadeDialog.open&&!$('applyCrossfade').disabled,'The crossfade preview did not calculate');
    check($('crossfadeStatus').textContent==='20 ms · 线性'&&$('crossfadeIn').getAttribute('points')!==$('crossfadeOut').getAttribute('points'),
      'The joined slices did not preview a linear transition');
    check(JSON.stringify(await api('project_read',{project_id:seamScore.id}))===JSON.stringify(seamScore),'The crossfade preview saved changes');
    crossfadeDialog.querySelector('[data-listen]').click();
    await until(()=>crossfadeAudition.binding()&&audio.auditionId===crossfadeAudition.binding().audition_id&&
      audio.auditionVariant===1&&!audio.paused,'The crossfade dialog did not play the draft',25000);
    const seamBinding=crossfadeAudition.binding();
    const seamReport=await api('arrangement_audition',{...seamBinding,action:'measure'});
    check(seamReport.transition_edges.frames.join(',')==='23520,24480'&&project.revision===seamScore.revision,
      'Manual crossfade audition moved the score or measured different boundaries');
    crossfadeDialog.querySelector('[data-side="A"]').click();
    await until(()=>audio.auditionId===seamBinding.audition_id&&audio.auditionVariant===0&&!audio.paused,
      'The crossfade dialog did not switch to its original');
    $('applyCrossfade').click();
    await until(()=>!crossfadeDialog.open&&project.revision===seamScore.revision+1,'Crossfade was not applied in one revision');
    const [fadeLeft,fadeRight]=clipGroupItems().map(item=>item.clip);
    check(fadeLeft.fade_out===960&&fadeRight.fade_in===960&&clipStartFrame(fadeRight)===23520&&fadeRight.offset===23520,
      'Crossfade moved the source signal on the timeline');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(seamScore.tracks),'Undo did not restore split positions and fades');
    report.desktop_crossfade={preview_without_write:true,source_samples_stay_in_place:true,one_undo:true,
      direct_ab_and_frozen_apply:true};
    const slipBase=project;
    const slipLane=slipBase.tracks.find(item=>item.id===seamLane);
    const slipClip=slipLane.clips[0];
    selectAudioClip(slipLane,slipClip);
    key(arrangeViewport,'s',{altKey:true});
    await until(()=>clipSlipDialog.open,'Alt+S did not open the audio slip dialog');
    const slipAmount=$('clipSlipAmount');
    slipAmount.value=-1;slipAmount.dispatchEvent(new Event('input',{bubbles:true}));
    await $('previewClipSlip').onclick();
    check($('applyClipSlip').disabled&&$('clipSlipStatus').textContent.includes('SLIP_OUT_OF_SOURCE'),
      'The source boundary did not prevent preview');
    slipAmount.value=10;slipAmount.dispatchEvent(new Event('input',{bubbles:true}));
    await $('previewClipSlip').onclick();
    check(!$('applyClipSlip').disabled&&project.revision===slipBase.revision,'The slip preview modified the score');
    clipSlipDialog.querySelector('[data-listen]').click();
    await until(()=>clipSlipAudition.binding(),'The audio slip A/B did not prepare',25000);
    const heardSlip=clipSlipAudition.binding();
    check((await invokeNative('verification_audition',{audition_id:heardSlip.audition_id})).distinct_pcm,
      'The slip A/B sides had identical audio');
    await $('applyClipSlip').onclick();
    await until(()=>!clipSlipDialog.open&&project.revision===slipBase.revision+1,'The slip was not applied');
    const changedSlip=project.tracks.find(item=>item.id===seamLane).clips[0];
    check(JSON.stringify(changedSlip)===JSON.stringify({...slipClip,offset:slipClip.offset+480}),
      'The slip changed the clip placement or other settings');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(slipBase.tracks),'One undo did not restore the source window');
    report.desktop_clip_slip={shortcut:true,preview_read_only:true,distinct_ab:true,same_placement:true,
      source_bounds_rejected:true,one_undo:true};
    let warpScore=await api('project_create',{title:'桌面片段跟速技术验证 · 非成品歌曲',bpm:120});
    warpScore=await api('composition_design',{project_id:warpScore.id,expected_revision:warpScore.revision,
      plan:{purpose:'technical_test',brief:'Verify clip tempo editing, loop bounds and model preview.'}});
    warpScore=await api('audio_import',{project_id:warpScore.id,expected_revision:warpScore.revision,
      filename:'跟速测试.wav',name:'跟速测试',data_base64:btoa(payload),start:0});
    await load(warpScore.id);
    const markWarp=()=>{const track=project.tracks.at(-1);selectAudioClip(track,trackClips(track)[0]);};
    markWarp();
    $('clipTempo').click();
    await until(()=>clipTempoDialog.open&&!$('applyClipTempo').disabled,'Clip tempo dialog did not calculate the existing span');
    $('clipTempoMode').value='beats';
    $('clipTempoMode').dispatchEvent(new Event('input'));
    $('clipTempoValue').value=4;
    $('clipTempoValue').dispatchEvent(new Event('input'));
    $('previewClipTempo').click();
    await until(()=>!$('applyClipTempo').disabled,'Target beat length did not preview');
    check($('clipTempoStatus').textContent==='4 拍 · 2.000 秒'&&JSON.stringify(await api('project_read',{project_id:warpScore.id}))===JSON.stringify(warpScore),
      'Tempo preview changed the project or displayed the wrong duration');
    $('applyClipTempo').click();
    await until(()=>!clipTempoDialog.open&&project.revision===warpScore.revision+1,'Did not save explicit clip beats');
    const originalWarped=trackClips(project.tracks.at(-1))[0];
    check(originalWarped.frames===48000&&originalWarped.beat_length===4&&clipEndBeat(originalWarped)===4&&project.format==='agent-vocal/7',
      'Tempo follow was not persisted');
    await edit([{op:'set_project',values:{bpm:180}}]);
    const moved=trackClips(project.tracks.at(-1))[0];
    check(clipEndBeat(moved)===4&&clipEndFrame(moved)===64000,'A project tempo edit did not change rendered time at the same beat');
    markWarp();
    $('loop').click();
    check(Math.abs(loopRange().end-64000/clipRate)<1e-8,'Looping did not include the stretched play time');
    $('loop').click();
    await history(false);
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(warpScore.tracks),'Undo did not restore the fixed-time source');
    report.desktop_tempo_follow={preview_without_write:true,beat_end_stable:true,render_duration_and_loop:true,undo_fixed:true};
    let alignedScore=await api('project_create',{title:'桌面对拍技术验证 · 非成品歌曲',bpm:120});
    alignedScore=await api('composition_design',{project_id:alignedScore.id,expected_revision:alignedScore.revision,
      plan:{purpose:'technical_test',brief:'Verify waveform editing and scoped source inspection.'}});
    const attackBytes=clipBytes.slice(),attackView=new DataView(attackBytes.buffer);
    for(let frame=0;frame<48000;frame++){
      let sample=Math.cos(frame/28)*350;
      for(const [start,frequency] of [[24000,.19],[36960,.34]]){
        const age=frame-start;
        if(age>=0&&age<1200)sample+=16000*Math.cos(age*frequency)*(1-age/1200);
      }
      attackView.setInt16(44+frame*2,Math.round(sample),true);
    }
    let attackPayload='';
    for(let i=0;i<attackBytes.length;i+=8192)attackPayload+=String.fromCharCode(...attackBytes.subarray(i,i+8192));
    alignedScore=await api('audio_import',{project_id:alignedScore.id,expected_revision:alignedScore.revision,
      filename:'对拍测试.wav',name:'对拍测试',data_base64:btoa(attackPayload),start:0});
    await load(alignedScore.id);
    const markAlign=()=>{const track=project.tracks.at(-1);selectAudioClip(track,trackClips(track)[0]);};
    markAlign();
    $('clipWarp').click();
    await until(()=>warpDialog.open&&$('warpPeaks').getAttribute('d')?.length>100,'Warp editor did not show the source waveform');
    $('addWarpPoint').click();
    check(warpPoints.length===1&&warpPoints[0].frame===24000&&warpPoints[0].beat===1,
      'Adding an anchor changed its source coordinate');
    $('warpRows').querySelector('button').click();
    const attackMarkers=[...$('warpAttackMarkers').querySelectorAll('[data-attack-frame]')];
    const attackAt=expected=>attackMarkers.find(marker=>Math.abs(Number(marker.dataset.attackFrame)-expected)<=120);
    const firstAttack=attackAt(24000),secondAttack=attackAt(36960);
    check(firstAttack&&secondAttack&&!$('warpAttacks').hidden,'The waveform did not locate both attacks');
    firstAttack.dispatchEvent(new MouseEvent('click',{bubbles:true}));
    $('warpAttacks').value=secondAttack.dataset.attackFrame;
    $('warpAttacks').dispatchEvent(new Event('change',{bubbles:true}));
    const firstFrame=Number(firstAttack.dataset.attackFrame);
    check(warpPoints.length===2&&warpPoints[0].frame===firstFrame&&
      warpPoints[1].frame===Number(secondAttack.dataset.attackFrame)&&project.revision===alignedScore.revision,
      'Attack tick and keyboard menu did not add exact source anchors without writing');
    $('warpRows').children[1].querySelector('button').click();
    const knob=$('warpLines').querySelector('circle');
    const dragTarget=new DOMPoint(12+408*.75/2,110).matrixTransform(warpPlot.getScreenCTM());
    const capture=warpPlot.setPointerCapture;
    try{
      warpPlot.setPointerCapture=()=>{};
      knob.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerId:11}));
      warpPlot.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,pointerId:11,clientX:dragTarget.x,clientY:dragTarget.y}));
      warpPlot.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,pointerId:11}));
    }finally{warpPlot.setPointerCapture=capture}
    check(warpPoints[0].frame===firstFrame&&warpPoints[0].beat===.75,'Dragging the beat did not preserve the source frame');
    $('previewWarp').click();
    await until(()=>!$('applyWarp').disabled,'Point preview did not return');
    check($('warpStatus').textContent==='1 个对拍点 · 2 拍'&&
      JSON.stringify(await api('project_read',{project_id:alignedScore.id}))===JSON.stringify(alignedScore),
      'Preview did not show the anchor, or wrote to the project');
    $('applyWarp').click();
    await until(()=>!warpDialog.open&&project.revision===alignedScore.revision+1&&project.format==='agent-vocal/8','Anchor did not apply in one revision');
    const alignedClip=trackClips(project.tracks.at(-1))[0];
    check(clipBeatAtSource(alignedClip,firstFrame)===.75&&clipSourceAtBeat(alignedClip,.75)===firstFrame,'Waveform map and inverse disagree');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(alignedScore.tracks),'Undo did not restore the original source window');
    report.desktop_warp={waveform_loaded:true,drag_keeps_source:true,preview_and_undo:true,display_mapping:true,
      attack_tick_and_menu:true};
    let consonantFixture=await api('project_create',{title:'桌面子音技术验证 · 非成品歌曲',bpm:120});
    const vocalId=consonantFixture.tracks[0].id;
    consonantFixture=await api('composition_design',{project_id:consonantFixture.id,expected_revision:consonantFixture.revision,
      plan:{purpose:'technical_test',brief:'Verify actual model consonant lead for one selected note.'}});
    consonantFixture=await api('project_edit',{project_id:consonantFixture.id,expected_revision:consonantFixture.revision,operations:[
      {op:'add_notes',track_id:vocalId,notes:[
        {id:'vowel',start:0,duration:2,pitch:60,lyric:'ア'},
        {id:'fricative',start:2,duration:1,pitch:62,lyric:'サ'}
      ]}
    ]});
    await load(consonantFixture.id);
    workspaceFocus='editor';
    chooseGroup(['vowel']);
    await $('readConsonant').onclick();
    check($('consonantMs').disabled&&$('consonantResult').textContent.includes('没有可调'),'A single vowel accepted lead timing');
    chooseGroup(['fricative']);
    await $('readConsonant').onclick();
    check(!$('consonantMs').disabled&&$('consonantResult').textContent.includes('s · 自动'),'The actual predicted fricative was not shown');
    $('consonantMs').value='200';
    $('consonantMs').dispatchEvent(new Event('change'));
    await until(()=>project.revision===consonantFixture.revision+1&&$('consonantResult').textContent.includes('当前 202.7'),
      'The inspector did not apply and analyze the edited lead');
    check(JSON.stringify(tr().notes[0])===JSON.stringify(consonantFixture.tracks[0].notes[0])&&
      tr().notes[1].consonant_ms===200&&tr().notes[1].start===2,'The inspector changed other current score data');
    await history(false);
    check(JSON.stringify(tr().notes)===JSON.stringify(consonantFixture.tracks[0].notes),'The edit could not be undone');
    report.desktop_pronunciation={actual_analysis:true,one_note_saved:true,one_undo:true};
    chooseGroup(['vowel']);
    await $('readConsonant').onclick();
    check(!$('vowelOffset').disabled&&$('consonantMs').disabled,'A vowel-only note needs only vowel timing');
    const vowelBase=project.revision;
    $('vowelOffset').value='80';
    $('vowelOffset').dispatchEvent(new Event('change'));
    await until(()=>project.revision===vowelBase+1&&$('consonantResult').textContent.includes('元音偏移 85.3 ms'),
      'The inspector did not report the applied vowel delay');
    check(project.format==='agent-vocal/10'&&tr().notes[0].start===0&&tr().notes[0].vowel_offset_ms===80,
      'The vowel edit changed the scored start or did not upgrade the format');
    await history(false);
    check(JSON.stringify(tr().notes)===JSON.stringify(consonantFixture.tracks[0].notes),'Vowel delay did not undo in one operation');
    report.desktop_pronunciation.vowel_offset_and_undo=true;
    const recordingFixture=await api('project_create',{title:'录音技术验证 · 非成品歌曲',bpm:120});
    await load(recordingFixture.id);
    await recordButton.onclick();
    check(recordDialog.open&&$('recordDevice').options.length>0,'The desktop did not enumerate any WASAPI input');
    const inputCount=$('recordDevice').options.length;
    const fixtureBinding={project_id:project.id,expected_revision:project.revision,start:1.25,track_id:null};
    recordingState=await captureCall('fixture_start',{binding:fixtureBinding,first_channel:1,channels:2});
    check(recordingState.frames===8820&&recordingState.sample_rate===44100&&recordingState.peak>.31,
      'Capture did not receive the stereo test signal');
    paintRecording();
    recordDialog.dispatchEvent(new Event('cancel',{cancelable:true}));
    await new Promise(resolve=>setTimeout(resolve,190));
    check(recordDialog.open,'Escape hid an active capture');
    await $('recordAction').onclick();
    const firstTake=recordingState.id;
    check(recordingState.phase==='ready'&&$('recordBeat').valueAsNumber===1.25,'Stopping did not retain the pending take placement');
    const original=await api('project_read',{project_id:recordingFixture.id});
    check(original.revision===fixtureBinding.expected_revision&&original.tracks.length===1,'Stop wrote to the project before confirmation');
    await $('recordAction').onclick();
    await until(()=>!recordDialog.open&&recordingState.phase==='idle'&&project.revision===fixtureBinding.expected_revision+1,'Import did not commit once');
    const recordedTrack=project.tracks.at(-1).id;
    check(project.tracks.at(-1).audio.asset==='audio_'+firstTake+'.wav'&&project.tracks.at(-1).audio.start===1.25,'Recorded take did not land at the selected beat');

    const appendBinding={project_id:project.id,expected_revision:project.revision,start:1.25,track_id:recordedTrack};
    await captureCall('fixture_start',{binding:appendBinding,first_channel:2,channels:1});
    const secondTake=(await captureCall('stop')).id;
    let collision=false;
    try{await captureCall('commit',{...appendBinding,take_id:secondTake})}
    catch(error){collision=error.message.includes('CLIP_OVERLAP')}
    check(collision&&(await captureCall('status')).id===secondTake,'An overlap did not preserve the take');
    await edit([{op:'set_project',values:{tail_seconds:.5}}]);
    let stale=false;
    try{await captureCall('commit',{...appendBinding,take_id:secondTake,start:2})}
    catch(error){stale=error.message.includes('REVISION_CONFLICT')}
    check(stale&&(await captureCall('status')).id===secondTake,'A revision conflict discarded the take');
    await recordButton.onclick();
    check(recordingState.id===secondTake&&$('recordTrack').value===recordedTrack,'Pending take could not reopen');
    $('recordBeat').value=2;
    await $('recordAction').onclick();
    await until(()=>!recordDialog.open&&recordingState.phase==='idle','The append dialog did not close');
    const recordedClips=trackClips(project.tracks.at(-1));
    check(recordedClips.length===2&&recordedClips[0].start===1.25&&recordedClips[1].start===2&&
      recordingState.phase==='idle','The second take did not append to the selected track');
    report.desktop_recording={windows_inputs:inputCount,confirmed_new_track:true,overlap_and_stale_preserved_take:true,
      repositioned_append:true,project_id:project.id,track_id:recordedTrack,assets:recordedClips.map(clip=>clip.asset)};
    await render(false);
    check(rendered?.project_id===project.id&&rendered.revision===project.revision,'Could not prepare backing for overdub');
    const syncBinding={project_id:project.id,expected_revision:project.revision,start:1.5,track_id:null};
    const syncArgs={binding:syncBinding,first_channel:1,channels:2,
      overdub:{job_id:rendered.job_id,lead_beats:1,correction_ms:0}};
    let wrongBacking=false;
    try{await captureCall('fixture_start',{...syncArgs,overdub:{...syncArgs.overdub,job_id:report.agent_transport_fixture.job_id}})}
    catch(error){wrongBacking=error.message.includes('当前版本')}
    check(wrongBacking&&(await captureCall('status')).phase==='idle','Outdated backing opened a capture');
    await recordButton.onclick();
    await $('recordOverdub').click();
    check(!$('recordSyncFields').hidden,'The overdub settings did not open');
    recordingState=await captureCall('fixture_start',syncArgs);
    paintRecording();
    let captureReady;
    for(let retry=0;retry<80;retry++){
      captureReady=await captureCall('status');
      if(captureReady.frames>=13230)break;
      await new Promise(resolve=>setTimeout(resolve,20));
    }
    check(captureReady.frames===13230&&captureReady.overdub&&!captureReady.preroll,'Output callback did not anchor the capture clock');
    const armed=await transport();
    check(armed.playing&&armed.capture_reserved&&armed.owner==='user','Accompaniment did not play under recording ownership');
    let seekBlocked=false;
    try{await invokeNative('transport',{command:'seek',seconds:0})}catch(error){seekBlocked=error.message.includes('录音期间')}
    check(seekBlocked,'Another transport command moved the backing during capture');
    const agentTransport=await api('desktop_transport',{action:'status'});
    let agentBlocked=false;
    try{await api('desktop_transport',{action:'play',ticket:agentTransport.ticket,project_id:project.id,
      expected_revision:project.revision,job_id:rendered.job_id,beat_range:[.5,1.5],reason:'Verify recording ownership'})}
    catch(error){agentBlocked=error.message.includes('USER_TRANSPORT_ACTIVE')}
    check(agentBlocked,'Agent acquired playback during recording');
    await $('recordAction').onclick();
    check(recordingState.phase==='ready'&&recordingState.offset_frames===4410&&Math.abs(recordingState.seconds-.2)<1e-12,
      'The aligned take did not remove the timestamped lead');
    const stopped=await transport();
    check(!stopped.playing&&!stopped.capture_reserved,'Stopping a take left accompaniment running');
    const alignedTake=recordingState.id;
    await $('recordAction').onclick();
    await until(()=>!recordDialog.open&&recordingState.phase==='idle','Overdub did not import');
    const alignedTrack=project.tracks.at(-1);
    check(alignedTrack.audio.start===1.5&&alignedTrack.audio.asset==='audio_'+alignedTake+'.wav','The overdub was not placed at its punch beat');
    report.desktop_overdub={stale_backing_rejected:true,output_clock_observed:true,seek_and_agent_blocked:true,
      playback_started_and_stopped:true,trimmed_input_frames:4410,project_id:project.id,
      track_id:alignedTrack.id,asset:alignedTrack.audio.asset,beat:1.5};
    await render(false);
    check(rendered?.revision===project.revision&&!!rendered.mix_url,'The metronome playback fixture needs the current mix');
    await until(()=>!document.querySelector('dialog[open]'),'Recording dialog did not close before click check');
    key(document.body,'m');
    await until(()=>$('metronomeToggle').getAttribute('aria-pressed')==='true','M did not enable the metronome');
    await invokeNative('transport',{command:'play',url:rendered.mix_url,seconds:0,loop:{start:0,end:.15}});
    let playingClick;
    for(let i=0;i<80;i++){
      playingClick=await transport();
      if(playingClick.click_pulses>=2&&playingClick.loop_wraps>0)break;
      await new Promise(resolve=>setTimeout(resolve,20));
    }
    check(playingClick.click_pulses>=2&&playingClick.loop_wraps>0&&playingClick.last_click_frame===0,
      'The native monitor click did not repeat on the loop boundary');
    await invokeNative('transport',{command:'stop'});
    key(document.body,'m');
    await until(()=>$('metronomeToggle').getAttribute('aria-pressed')==='false','M did not turn the metronome off');
    let countFixture=await api('project_create',{title:'预备拍录音验证 · 非成品歌曲',bpm:120});
    countFixture=await api('composition_design',{project_id:countFixture.id,expected_revision:countFixture.revision,
      plan:{purpose:'technical_test',brief:'Observe output-clock anchored click-only recording at beat zero.'}});
    countFixture=await api('project_edit',{project_id:countFixture.id,expected_revision:countFixture.revision,
      operations:[{op:'set_project',values:{meter:[6,8]}}]});
    await load(countFixture.id);
    await recordButton.onclick();
    $('recordOverdub').checked=false;
    $('recordClick').click();
    check($('recordClick').checked&&!$('recordSyncFields').hidden,'Click-only controls did not appear');
    const countBinding={project_id:countFixture.id,expected_revision:countFixture.revision,start:0,track_id:null};
    recordingState=await captureCall('fixture_start',{binding:countBinding,first_channel:1,channels:2,
      overdub:{click:true,lead_beats:3,correction_ms:0}});
    let negativeClick=false,clickAtPunch=false;
    for(let retry=0;retry<125;retry++){
      const state=await transport();
      negativeClick||=state.last_click_frame<0&&state.seconds===0&&state.capture_reserved;
      clickAtPunch=state.click_pulses>=3&&state.last_click_frame===0&&state.seconds>0&&state.peak.some(peak=>peak>.015);
      if(clickAtPunch)break;
      await new Promise(resolve=>setTimeout(resolve,20));
    }
    check(negativeClick&&clickAtPunch,'The pre-zero count-in or punch click did not reach native output');
    await $('recordAction').onclick();
    check(recordingState.phase==='ready'&&recordingState.offset_frames===4410,
      'The click-only take did not align to the same output timestamp');
    await $('recordAction').onclick();
    await until(()=>!recordDialog.open&&project.revision===countFixture.revision+1,'The click-only take did not import');
    check(project.tracks.at(-1).audio?.start===0,'The count-in shifted the saved take');
    check(!(await transport()).capture_reserved,'Commit left the capture clock reserved');
    report.desktop_metronome={keyboard_and_loop:true,click_only_controls:true,negative_preroll:true,
      punch_output_peak:true,aligned_at_zero:true,asset:project.tracks.at(-1).audio.asset};
    let phraseFixture=await api('project_create',{title:'乐句复核技术验证 · 非成品歌曲',bpm:120});
    const phraseVoice=phraseFixture.tracks[0].id;
    phraseFixture=await api('composition_design',{project_id:phraseFixture.id,expected_revision:phraseFixture.revision,
      plan:{purpose:'technical_test',brief:'Check reference phrase, register, backing onsets and frozen revisions.',
        sections:[{name:'第一段',start:0,end:2.5,role:'核对主唱与琴声入场'},
          {name:'第二段',start:2.5,end:3.5,role:'核对第二句'}]}});
    phraseFixture=await api('project_edit',{project_id:phraseFixture.id,expected_revision:phraseFixture.revision,operations:[
      {op:'add_track',track_ref:'keys',values:{name:'测试键盘',instrument:{program:0}}},
      {op:'add_notes',track_id:phraseVoice,notes:[
        {id:'p_first',start:0,duration:.5,pitch:60,lyric:'サ'},
        {id:'p_jump',start:.5,duration:.5,pitch:72,lyric:'カ'},
        {id:'p_hold',start:1,duration:.5,pitch:67,lyric:'イ'},
        {id:'p_next',start:2.5,duration:.5,pitch:65,lyric:'ア'}
      ]},
      {op:'add_notes',track_ref:'keys',notes:[{start:.25,duration:.5,pitch:52},{start:1.75,duration:.25,pitch:55},{start:2.25,duration:.25,pitch:59}]}
    ]});
    await load(phraseFixture.id);
    await switchArrangement(true);
    const phraseKeys=phraseFixture.tracks[1].id,selectPhrase=()=>updateArrangeSelection({start:0,end:3.5,ids:[phraseVoice,phraseKeys]});
    selectPhrase();
    const review=await api('composition_review',{project_id:phraseFixture.id,expected_revision:phraseFixture.revision,
      track_ids:[phraseVoice,phraseKeys],beat_range:[0,3.5]});
    const firstPhrase=review.score_detail.phrases[0];
    check(firstPhrase.boundary_note_ids.join(',')==='p_first,p_hold'&&firstPhrase.gap_after.seconds===.5&&
      firstPhrase.gap_after.backing.pitched_attacks===2&&review.score_detail.sections[1].vocal_onsets===1,'Read-only review lost the phrase or backing evidence');
    const releaseIndex=firstPhrase.listening_points.findIndex(point=>point.roles.includes('release'));
    const releasePoint=firstPhrase.listening_points[releaseIndex];
    check(releasePoint.beat===1.5&&releasePoint.note_ids[0]==='p_hold'&&
      releasePoint.backing.on_or_after.offset_ms===125&&releasePoint.backing.on_or_after.tracks[0].track_id===phraseKeys,
      'The first backing entrance was not aligned to the scored phrase release');
    await $('reviewSong').onclick();
    check(knowledgeDialog.open&&$('knowledgeBody').textContent.includes('サ カ イ')&&
      $('knowledgeBody').textContent.includes('伴奏 2 次入音'),'The review panel did not show the local score evidence');
    const releaseButton=$('knowledgeBody').querySelector(`[data-listening-phrase="0"][data-listening-index="${releaseIndex}"]`);
    check(releaseButton?.textContent.includes('125 ms')&&releaseButton.textContent.includes('测试键盘'),
      'The exact backing entrance was not visible');
    releaseButton.click();
    await until(()=>!knowledgeDialog.open&&workspaceFocus==='editor'&&picked()[0]?.id==='p_hold'&&cursorBeat===1.5,
      'The release marker did not focus the selected note and exact release beat');
    await switchArrangement(true);
    selectPhrase();
    await $('reviewSong').onclick();
    $('knowledgeBody').querySelector('[data-review-index="0"]').click();
    await until(()=>!knowledgeDialog.open&&arrangeSelection?.ids.length===1&&arrangeSelection.end===1.5,
      'The review did not locate the exact phrase in the arrangement');
    check(trackId===phraseVoice&&cursorBeat===0,'The review focused the wrong track or start');
    report.desktop_music_review={scoped_evidence_and_focus:true,exact_release_and_backing:true};
    let harmonyFixture=await api('project_create',{title:'和弦标记验证 · 非成品歌曲',bpm:120});
    const markingVoice=harmonyFixture.tracks[0].id;
    harmonyFixture=await api('composition_design',{project_id:harmonyFixture.id,expected_revision:harmonyFixture.revision,
      plan:{purpose:'technical_test',brief:'Verify editable chord annotations without changing scored audio.'}});
    harmonyFixture=await api('project_edit',{project_id:harmonyFixture.id,expected_revision:harmonyFixture.revision,operations:[
      {op:'add_notes',track_id:markingVoice,notes:[{id:'held',start:.5,duration:2,pitch:65,lyric:'ア'}]}
    ]});
    await load(harmonyFixture.id);
    await switchArrangement(true);
    updateArrangeSelection(null);
    const oldEngine=$('renderEngine').value;
    $('renderEngine').value='guide';
    await render(false);
    const harmonyBeforeJob=rendered.job_id;
    const startingHarmony=project.revision;
    await harmonyButton.onclick();
    check(harmonyDialog.open&&harmonyDraft.points.length===0,'Empty chord timeline did not open');
    $('harmonyAdd').click();
    let firstMark=$('harmonyRows').firstElementChild;
    firstMark.querySelector('[data-field=symbol]').value='Cmaj7/E';
    firstMark.querySelector('[data-field=symbol]').dispatchEvent(new Event('input',{bubbles:true}));
    await $('harmonyApply').onclick();
    await until(()=>!harmonyDialog.open&&project.format==='agent-vocal/12'&&project.revision===startingHarmony+1&&arrangeHeader===60,
      'Adding a chord did not render its ruler row');
    check(project.harmony[0].symbol==='Cmaj7/E','The chord symbol was not persisted');
    await edit([{op:'set_project',values:{harmony:[{beat:0,symbol:'Cmaj7/E'},{beat:2,symbol:'Dm9/F'},{beat:4,symbol:'G7(b9,#11)'},{beat:6,symbol:'N.C.'}]}}]);
    const rangeReview=await api('composition_review',{project_id:project.id,expected_revision:project.revision,
      track_ids:[markingVoice],beat_range:[0,3]});
    const heldChecks=rangeReview.score_detail.phrases[0].harmony_listen_points;
    check(heldChecks.length===1&&heldChecks[0].symbol==='Cmaj7/E'&&heldChecks[0].beat_range.join(',')==='0.5,2',
      'The held annotation did not stop at the chord change');
    updateArrangeSelection({start:1,end:3,ids:[markingVoice]});
    await harmonyButton.onclick();
    check(harmonyDraft.points[0].beat===1&&harmonyDraft.points[0].symbol==='Cmaj7/E'&&
      $('harmonyRows').firstElementChild.querySelector('[data-field=beat]').disabled,'Scoped edit lost its carried opening chord');
    for(const [index,symbol] of ['B♭maj7/D','G7sus4/A'].entries()){
      const field=$('harmonyRows').children[index].querySelector('[data-field=symbol]');
      field.value=symbol;
      field.dispatchEvent(new Event('input',{bubbles:true}));
    }
    const initialRangeRevision=project.revision;
    await $('harmonyApply').onclick();
    await until(()=>!harmonyDialog.open&&project.revision===initialRangeRevision+1,'The scoped harmony edit did not save once');
    check(project.harmony.map(point=>point.beat+':'+point.symbol).join(',')==='0:Cmaj7/E,1:B♭maj7/D,2:G7sus4/A,3:Dm9/F,4:G7(b9,#11),6:N.C.',
      'Changing a chord range did not restore the following harmony');
    await render(false);
    const harmonyAfterJob=rendered.job_id;
    $('renderEngine').value=oldEngine;
    report.desktop_harmony={timeline_and_scoped_edit:true,precise_chord_listen_point:true,
      project_id:project.id,new_revision:project.revision,render_jobs:[harmonyBeforeJob,harmonyAfterJob]};
    let voicingFixture=await api('project_create',{title:'声部配置技术验证 · 非成品歌曲',bpm:120});
    const voicingVoice=voicingFixture.tracks[0].id;
    voicingFixture=await api('composition_design',{project_id:voicingFixture.id,expected_revision:voicingFixture.revision,
      plan:{purpose:'technical_test',brief:'Verify two written voicings over explicit independent rhythms.'}});
    voicingFixture=await api('project_edit',{project_id:voicingFixture.id,expected_revision:voicingFixture.revision,operations:[
      {op:'set_project',values:{harmony:[{beat:0,symbol:'Cmaj7/E'},{beat:2,symbol:'G7/D'},{beat:4,symbol:'Fmaj9/A'},{beat:6,symbol:'N.C.'}]}},
      {op:'add_notes',track_id:voicingVoice,notes:[
        {start:.5,duration:1.5,pitch:67,lyric:'ア'},{start:2.25,duration:1,pitch:72,lyric:'イ'},
        {start:4.25,duration:1.5,pitch:69,lyric:'ウ'}
      ]},
      {op:'add_track',track_ref:'keys',values:{name:'配置测试键盘',instrument:{program:0}}},
      {op:'add_track',track_ref:'bass',values:{name:'配置测试低音',instrument:{program:33}}}
    ]});
    await load(voicingFixture.id);
    let routingFixture=await api('project_create',{title:'共享返回技术验证 · 非成品歌曲',bpm:120});
    routingFixture=await api('composition_design',{project_id:routingFixture.id,expected_revision:routingFixture.revision,
      plan:{purpose:'technical_test',brief:'Measure shared returns on native playback and compare two wet-only mixes.'}});
    routingFixture=await api('audio_import',{project_id:routingFixture.id,expected_revision:routingFixture.revision,
      name:'返回测试信号',filename:'返回验证.wav',data_base64:btoa(payload),start:0});
    const returnSource=routingFixture.tracks.at(-1).id;
    await load(routingFixture.id);
    if(mixerPanel.hidden)$('showMixer').click();
    await until(()=>!mixerPanel.hidden&&!routingButton.hidden,'The routing entry did not appear in the mixer');
    routingButton.click();
    await until(()=>routingDialog.open,'The routing dialog did not open');
    $('routingDelay').click();
    const returnId=$('routingBus').value;
    const routingInput=(type)=>routingDialog.querySelector('.routing-row[data-track="'+returnSource+'"] input[type='+type+']');
    routingInput('checkbox').click();
    check(project.revision===routingFixture.revision,'Creating a return wrote before Apply');
    $('routingApply').click();
    await until(()=>!routingDialog.open&&project.revision===routingFixture.revision+1&&returnViews.has(returnId),
      'The return and send did not save together');
    check(project.format==='agent-vocal/13'&&project.aux_buses[0].effects[0].mix===1&&project.tracks.at(-1).sends[0].bus_id===returnId,
      'The UI created the wrong routing or a mixed dry return');
    selectAudioClip(project.tracks.at(-1),trackClips(project.tracks.at(-1))[0]);
    if(!looping)$('loop').click();
    cursorBeat=0;
    await render(true);
    const returnMeter=()=>audio.trackMeters[audio.monitorTrackIds.indexOf(returnId)]?.[0]||0;
    await until(()=>audio.liveMix&&!audio.paused&&returnMeter()>.012,'The WASAPI return meter did not receive the tone');
    const returnBefore=returnMeter(),beforeRoutingTransport=await transport(),beforeRoutingRevision=project.revision;
    routingButton.click();
    await until(()=>routingDialog.open,'Cannot reopen the send control');
    function sendValue(value){
      const input=routingInput('number');
      input.value=value;input.dispatchEvent(new Event('input',{bubbles:true}));
    }
    sendValue(-30);
    await until(()=>returnMeter()>returnBefore*.19&&returnMeter()<returnBefore*.34,'Draft did not lower the return signal');
    const draftRatio=20*Math.log10(returnMeter()/returnBefore);
    check(project.revision===beforeRoutingRevision&&draftRatio>-15&&draftRatio<-9,
      'Live send draft altered the project or missed the expected level');
    $('closeRouting').click();
    await until(()=>returnMeter()>returnBefore*.85,'Closing the send draft did not restore the return');
    routingButton.click();await until(()=>routingDialog.open,'Cannot reopen the sends');
    sendValue(-24);
    $('routingApply').click();
    await until(()=>!routingDialog.open&&project.revision===beforeRoutingRevision+1&&returnMeter()<returnBefore*.68&&returnMeter()>.005,
      'The saved send did not reach the active return');
    const afterRoutingTransport=await transport();
    check(afterRoutingTransport.playing&&afterRoutingTransport.live_mix&&afterRoutingTransport.url===beforeRoutingTransport.url&&
      afterRoutingTransport.audio_frames>beforeRoutingTransport.audio_frames&&afterRoutingTransport.mix_changes>beforeRoutingTransport.mix_changes,
      'Saving the send interrupted native playback');
    returnViews.get(returnId).name.click();
    await until(()=>fxDialog.open&&$('studioFxScope').value==='aux:'+returnId,'The return did not open its insert editor');
    const delayParam=fxDialog.querySelector('[data-param=time_ms]');
    delayParam.value=160;delayParam.dispatchEvent(new Event('input',{bubbles:true}));
    $('studioFxSave').click();
    await until(()=>!fxDialog.open&&project.aux_buses[0].effects[0].time_ms===160,'Could not commit a return effect');
    const afterEffectsTransport=await transport();
    check(afterEffectsTransport.playing&&afterEffectsTransport.live_mix&&afterEffectsTransport.url===beforeRoutingTransport.url,
      'Saving return inserts cleared the native mixer');
    audio.pause();
    await until(()=>audio.paused,'Cannot stop the routing fixture');
    report.desktop_routing={project_id:project.id,return_id:returnId,send_return_one_edit:true,live_send_db:draftRatio,
      draft_restored:true,continuous_native_playback:true,return_insert_saved:true};
    const importedGroupTrack=await api('audio_import',{project_id:project.id,expected_revision:project.revision,
      name:'第二编组成员',filename:'第二编组信号.wav',data_base64:btoa(payload),start:0});
    const secondMember=importedGroupTrack.tracks.at(-1).id;
    const groupedLevel=await api('project_edit',{project_id:project.id,expected_revision:importedGroupTrack.revision,
      operations:[{op:'update_track',track_id:secondMember,values:{gain_db:-12}}]});
    await load(groupedLevel.id);
    if(mixerPanel.hidden)$('showMixer').click();
    await until(()=>!mixerPanel.hidden&&!routingButton.hidden,'Mix group routing control missing');
    routingButton.click();
    await until(()=>routingDialog.open,'Cannot open mix group routing');
    $('routingGroup').click();
    const mixGroupId=$('routingBus').value;
    const membership=id=>routingDialog.querySelector('.routing-row[data-track="'+id+'"] input[type=checkbox]');
    for(const id of [returnSource,secondMember])membership(id).click();
    check(project.revision===groupedLevel.revision,'Creating the group wrote before Apply');
    $('routingApply').click();
    await until(()=>!routingDialog.open&&project.revision===groupedLevel.revision+1&&returnViews.has(mixGroupId),
      'The group was not saved with its two members');
    check(project.format==='agent-vocal/14'&&project.tracks.filter(t=>t.output_group===mixGroupId).length===2,
      'Wrong group schema or membership');
    const groupedSource=project.tracks.find(t=>t.id===returnSource);
    selectAudioClip(groupedSource,trackClips(groupedSource)[0]);
    if(!looping)$('loop').click();
    cursorBeat=0;
    await render(true);
    const readBus=id=>audio.trackMeters[audio.monitorTrackIds.indexOf(id)]?.[0]||0;
    await until(()=>audio.liveMix&&!audio.paused&&readBus(mixGroupId)>.19,'Group did not feed the real-time output');
    const baseGroupPeak=readBus(mixGroupId),baseGrouped=await transport(),startGroupRevision=project.revision;
    routingButton.click();await until(()=>routingDialog.open,'Cannot reopen group levels');
    const trimGroup=routingDialog.querySelector('input[aria-label="编组电平 dB"]');
    trimGroup.value=-12;trimGroup.dispatchEvent(new Event('input',{bubbles:true}));
    await until(()=>readBus(mixGroupId)>.19*baseGroupPeak&&readBus(mixGroupId)<.32*baseGroupPeak,
      'Group gain draft did not reach the meter');
    const groupGainMeasured=20*Math.log10(readBus(mixGroupId)/baseGroupPeak);
    check(readBus(returnSource)>.12&&project.revision===startGroupRevision,
      'The group gain draft also edited source tracks');
    $('closeRouting').click();
    await until(()=>readBus(mixGroupId)>.9*baseGroupPeak,'Cancelling the group draft did not restore its level');
    routingButton.click();await until(()=>routingDialog.open,'Cannot reopen group membership');
    membership(secondMember).click();
    await until(()=>readBus(mixGroupId)>.15&&readBus(mixGroupId)<.87*baseGroupPeak,'Sending the second track to master did not update the group');
    $('routingApply').click();
    await until(()=>!routingDialog.open&&project.revision===startGroupRevision+1&&project.tracks.at(-1).output_group===undefined,
      'The track did not save its direct master route');
    const afterGroupRoute=await transport();
    check(afterGroupRoute.playing&&afterGroupRoute.live_mix&&afterGroupRoute.url===baseGrouped.url&&
      afterGroupRoute.audio_frames>baseGrouped.audio_frames,'Saving group membership interrupted playback');
    returnViews.get(mixGroupId).name.click();
    await until(()=>fxDialog.open&&$('studioFxScope').value==='group:'+mixGroupId,'The group insert editor did not open');
    $('studioFxKind').value='compressor';$('studioFxAdd').click();
    $('studioFxSave').click();
    await until(()=>!fxDialog.open&&project.mix_groups[0].effects[0]?.kind==='compressor','Group compression did not save');
    check((await transport()).playing,'Saving a group effect interrupted playback');
    audio.pause();
    await until(()=>audio.paused,'Cannot stop the grouping fixture');
    report.desktop_mix_groups={project_id:project.id,group_id:mixGroupId,group_and_members_one_edit:true,
      gain_draft_and_cancel:true,measured_gain_db:groupGainMeasured,continuous_membership_edit:true,group_insert_saved:true};
    const peakSignal=new Uint8Array(clipBytes);
    const peakData=new DataView(peakSignal.buffer);
    for(let index=0;index<48000;index++){
      const fade=Math.min(1,index/960,(48000-index)/960);
      peakData.setInt16(44+index*2,Math.round(7500*fade*Math.sin(index*Math.PI/2+Math.PI/4)),true);
    }
    let peakPayload='';
    for(let index=0;index<peakSignal.length;index+=8192)peakPayload+=String.fromCharCode(...peakSignal.subarray(index,index+8192));
    let peakFixture=await api('project_create',{title:'导出真峰值实测 · 非成品歌曲',bpm:120});
    peakFixture=await api('composition_design',{project_id:peakFixture.id,expected_revision:peakFixture.revision,
      plan:{purpose:'technical_test',brief:'Verify intersample limiting and file-based desktop playback.'}});
    peakFixture=await api('audio_import',{project_id:peakFixture.id,expected_revision:peakFixture.revision,
      name:'峰值测量信号',filename:'峰值测量.wav',data_base64:btoa(peakPayload),start:0});
    const sibilantSignal=new Uint8Array(clipBytes);
    const sibilantData=new DataView(sibilantSignal.buffer);
    for(let index=0;index<48000;index++){
      const fade=Math.min(1,index/960,(48000-index)/960);
      sibilantData.setInt16(44+index*2,Math.round(9000*fade*Math.sin(index*2*Math.PI*11873/48000)),true);
    }
    let sibilantPayload='';
    for(let i=0;i<sibilantSignal.length;i+=8192)sibilantPayload+=String.fromCharCode(...sibilantSignal.subarray(i,i+8192));
    let deEsserFixture=await api('project_create',{title:'齿音实时监听 · 非成品歌曲',bpm:120});
    deEsserFixture=await api('composition_design',{project_id:deEsserFixture.id,expected_revision:deEsserFixture.revision,
      plan:{purpose:'technical_test',brief:'Measure a steady high tone through the active de-esser and device clock.'}});
    deEsserFixture=await api('audio_import',{project_id:deEsserFixture.id,expected_revision:deEsserFixture.revision,
      name:'高频监听信号',filename:'齿音监听.wav',data_base64:btoa(sibilantPayload),start:0});
    await load(deEsserFixture.id);
    const sibilantTrack=project.tracks.at(-1);
    selectAudioClip(sibilantTrack,trackClips(sibilantTrack)[0]);
    if(!looping)$('loop').click();
    cursorBeat=0;
    await render(true);
    await until(()=>!audio.paused&&audio.liveMix,'Could not start the de-esser fixture');
    await new Promise(resolve=>setTimeout(resolve,210));
    const drySibilance=await transport();
    key(arrangeViewport,'F3');
    await until(()=>fxDialog.open,'Could not open de-esser controls');
    $('studioFxKind').value='de_esser';$('studioFxAdd').click();
    check(fxDialog.querySelector('.studio-fx-insert-title strong')?.textContent==='齿音控制',
      'The effect picker did not add the de-esser');
    await new Promise(resolve=>setTimeout(resolve,540));
    const wetSibilance=await transport();
    const attenuation=20*Math.log10(wetSibilance.peak[0]/drySibilance.peak[0]);
    check(attenuation<-3&&attenuation>-9&&wetSibilance.playing&&wetSibilance.mix_changes>drySibilance.mix_changes&&
      wetSibilance.url===drySibilance.url&&wetSibilance.audio_frames>drySibilance.audio_frames,
      `De-esser applied ${attenuation} dB or restarted the device clock`);
    check(project.revision===deEsserFixture.revision,'The live de-esser draft wrote to the project');
    $('studioFxClose').click();
    await until(()=>!fxDialog.open,'The de-esser draft did not close');
    await new Promise(resolve=>setTimeout(resolve,370));
    const revertedSibilance=await transport();
    check(revertedSibilance.playing&&revertedSibilance.peak[0]>.86*drySibilance.peak[0],
      'Closing a de-esser draft did not restore the dry signal');
    $('stop').click();
    if(looping)$('loop').click();
    report.desktop_de_esser={live_reduction_db:attenuation,continuous_device_clock:true,draft_reverted:true};
    {
      const expansionBase=project.revision;
      const expansionTrack=project.tracks.at(-1).id;
      if(!looping)$('loop').click();
      cursorBeat=0;
      await render(true);
      await until(()=>!audio.paused&&audio.liveMix,'Could not start the expander fixture');
      await new Promise(resolve=>setTimeout(resolve,180));
      const dryExpansion=await transport();
      key(arrangeViewport,'F3');
      await until(()=>fxDialog.open,'Could not open live expansion controls');
      $('studioFxKind').value='expander';$('studioFxAdd').click();
      check(fxDialog.querySelector('.studio-fx-insert-title strong')?.textContent==='向下扩展',
        'The effect picker did not add an expander');
      for(const [param,value] of [['threshold_db',-3],['knee_db',0],['hold_ms',0],['release_ms',45]]){
        const input=fxDialog.querySelector('[data-param="'+param+'"]');
        input.value=value;input.dispatchEvent(new Event('input',{bubbles:true}));
      }
      await new Promise(resolve=>setTimeout(resolve,470));
      const wetExpansion=await transport();
      const expansionDb=20*Math.log10(wetExpansion.peak[0]/dryExpansion.peak[0]);
      check(expansionDb<-6&&expansionDb>-10&&wetExpansion.playing&&
        wetExpansion.mix_changes>dryExpansion.mix_changes&&wetExpansion.url===dryExpansion.url&&
        wetExpansion.audio_frames>dryExpansion.audio_frames,'Expansion did not reduce the signal on the same device clock: '+expansionDb);
      check(project.revision===expansionBase,'Auditioning expansion wrote to the project');
      $('studioFxSave').click();
      await until(()=>!fxDialog.open&&project.revision===expansionBase+1,'Live expansion did not save');
      check(project.format==='agent-vocal/20'&&project.tracks.at(-1).effects[0].hold_ms===0&&
        project.tracks.at(-1).effects[0].release_ms===45&&!audio.paused&&audio.liveMix,'Expansion save changed the clock or lost settings');
      $('stop').click();if(looping)$('loop').click();
      report.desktop_expander={live_reduction_db:expansionDb,continuous_device_clock:true,draft_and_save:true};
    }
    let duckFixture=await api('project_create',{title:'旁链实时监听 · 非成品歌曲',bpm:120});
    duckFixture=await api('composition_design',{project_id:duckFixture.id,expected_revision:duckFixture.revision,
      plan:{purpose:'technical_test',brief:'Use a muted input to duck another track on the WASAPI device.'}});
    duckFixture=await api('audio_import',{project_id:duckFixture.id,expected_revision:duckFixture.revision,
      name:'被压音轨',filename:'持续监听.wav',data_base64:btoa(sibilantPayload),start:0});
    const duckTarget=duckFixture.tracks.at(-1).id;
    duckFixture=await api('audio_import',{project_id:duckFixture.id,expected_revision:duckFixture.revision,
      name:'触发轨',filename:'闪避触发.wav',data_base64:btoa(peakPayload),start:0});
    const duckSource=duckFixture.tracks.at(-1).id;
    duckFixture=await api('project_edit',{project_id:duckFixture.id,expected_revision:duckFixture.revision,
      operations:[{op:'update_track',track_id:duckSource,values:{mute:true,gain_db:-40}}]});
    await load(duckFixture.id);
    const duckTrack=project.tracks.find(t=>t.id===duckTarget);
    selectAudioClip(duckTrack,trackClips(duckTrack)[0]);
    if(!looping)$('loop').click();
    cursorBeat=.3;
    await render(true);
    await until(()=>!audio.paused&&audio.liveMix,'Could not start sidechain dry playback');
    await new Promise(resolve=>setTimeout(resolve,220));
    const dryDuck=await transport();
    key(arrangeViewport,'F3');
    await until(()=>fxDialog.open&&$('studioFxScope').value===duckTarget,'F3 did not open the destination');
    $('studioFxKind').value='ducker';$('studioFxAdd').click();
    const sourcePick=fxDialog.querySelector('[data-param=source_track_id]');
    check(sourcePick&&[...sourcePick.options].every(option=>option.value!==duckTarget),
      'The destination track was available as its own trigger');
    sourcePick.value=duckSource;sourcePick.dispatchEvent(new Event('change',{bubbles:true}));
    await new Promise(resolve=>setTimeout(resolve,560));
    const wetDuck=await transport();
    const duckDb=20*Math.log10(wetDuck.peak[0]/dryDuck.peak[0]);
    check(duckDb<-4&&duckDb>-7.5&&wetDuck.playing&&wetDuck.mix_changes>dryDuck.mix_changes&&
      wetDuck.url===dryDuck.url&&wetDuck.audio_frames>dryDuck.audio_frames,
      `Sidechain applied ${duckDb} dB or changed the audio device clock`);
    check(project.revision===duckFixture.revision,'The sidechain draft wrote ahead of approval');
    $('studioFxSave').click();
    await until(()=>!fxDialog.open&&project.revision===duckFixture.revision+1,
      'The live sidechain could not be committed');
    const savedDuck=project.tracks.find(t=>t.id===duckTarget).effects[0];
    check(project.format==='agent-vocal/17'&&savedDuck.source_track_id===duckSource&&!audio.paused&&audio.liveMix,
      'The sidechain save lost routing or ended live playback');
    $('stop').click();if(looping)$('loop').click();
    report.desktop_sidechain={continuous_device_clock:true,saved_explicit_source:true,
      muted_trigger:true,live_reduction_db:duckDb};
    await load(peakFixture.id);
    key(arrangeViewport,'F3');
    await until(()=>fxDialog.open,'Could not open the true-peak limiter');
    $('studioFxScope').value='master';$('studioFxScope').dispatchEvent(new Event('change',{bubbles:true}));
    $('studioFxMasterEnabled').click();
    const detectorChoice=fxDialog.querySelector('[data-param=peak_mode]');
    check(detectorChoice.value==='sample_peak','New limiter did not preserve the sample-peak default');
    detectorChoice.value='true_peak';
    detectorChoice.dispatchEvent(new Event('change',{bubbles:true}));
    const outputCeiling=fxDialog.querySelector('[data-param=ceiling_db]');
    check(outputCeiling.closest('label').textContent.includes('dBTP')&&$('studioFxMessage').textContent.includes('导出用真峰值'),
      'True-peak selection did not distinguish the live monitor and export');
    outputCeiling.value=-18;
    outputCeiling.dispatchEvent(new Event('input',{bubbles:true}));
    $('studioFxPreview').click();
    await until(()=>!fxDialog.open&&project.revision===peakFixture.revision+1&&!audio.paused&&audio.liveMix,
      'Apply and audition did not start live monitoring with true-peak export enabled',20000);
    const peakJobs=await api('render_jobs',{project_id:project.id,expected_revision:project.revision});
    check(!rendered&&!peakJobs.jobs.length,'Live playback with true-peak export created a render');
    await playTruePeakMix();
    await until(()=>rendered?.master_limiter?.measurement==='true_peak'&&!audio.paused&&!audio.liveMix,
      'Explicit true-peak audition did not play the rendered file',20000);
    const peakExport=rendered;
    const peakTransport=await transport();
    check(peakExport.loudness.true_peak_dbtp<=-18&&peakExport.loudness.true_peak_dbtp>-18.55&&
      !peakTransport.live_mix&&peakTransport.url===peakExport.mix_url&&project.format==='agent-vocal/15',
      'The editor did not use the reported true-peak file');
    let wrongGraphRejected=false;
    try{await audio.monitorMix(project,peakExport.job_id,{tracks:[]})}
    catch(error){wrongGraphRejected=error.message.includes('真峰值混音')}
    check(wrongGraphRejected&&(await transport()).playing&&!audio.liveMix,
      'Live sample peak took over the rendered true-peak audition');
    cursorBeat=.5;
    await edit([{op:'set_project',values:{tail_seconds:1.9}}]);
    await until(()=>audio.paused,'A project edit kept playing a stale true-peak render');
    check(cursorBeat===.5&&!(await transport()).url,'True-peak invalidation moved the edit cursor or retained old audio');
    report.desktop_true_peak={project_id:project.id,enable_and_play_one_edit:true,played_rendered_wav:true,
      live_sample_graph_rejected:true,stale_audio_cleared:true,output_dbtp:peakExport.loudness.true_peak_dbtp};
    let motiveFixture=await api('project_create',{title:'动机回归技术验证 · 非成品歌曲',bpm:120});
    const motifVoice=motiveFixture.tracks[0].id;
    motiveFixture=await api('composition_design',{project_id:motiveFixture.id,expected_revision:motiveFixture.revision,
      plan:{purpose:'technical_test',brief:'Compare a reference and its two scored returns.'}});
    const motifNotes=(tag,beat,shift,scale,variant=false)=>{
      const lyrics=variant?['サ','ビ','ヲ','フ','ク']:['ユ','キ','ヲ','フ','ム'];
      return [60,64,63,67,65].map((pitch,index)=>({
        id:'motif_'+tag+index,start:beat+[0,.375,.875,1.25,variant?1.875:1.75][index]*scale,
        duration:[.25,.375,.25,.375,variant?.75:.625][index]*scale,
        pitch:pitch+shift+(variant&&index===4?5:0),lyric:lyrics[index],velocity:72
      }));
    };
    motiveFixture=await api('project_edit',{project_id:motiveFixture.id,expected_revision:motiveFixture.revision,operations:[
      {op:'set_project',values:{harmony:[{beat:0,symbol:'Cmaj7'},{beat:4,symbol:'Dm9'},{beat:8,symbol:'Fmaj7'}]}},
      {op:'add_notes',track_id:motifVoice,notes:[...motifNotes('a',0,0,1),...motifNotes('b',4,2,1,true),...motifNotes('c',8,5,.5)]},
      {op:'set_lyric_line',track_id:motifVoice,note_ids:[0,1,2,3,4].map(i=>'motif_b'+i),text:'錆を拭く'},
      {op:'add_track',track_ref:'keys',values:{name:'回归处钢琴',instrument:{program:0}}},
      {op:'add_notes',track_ref:'keys',notes:[{start:4.25,duration:.25,pitch:53},{id:'motif_key_reply',start:5.5,duration:.5,pitch:57},
        {start:6.875,duration:.25,pitch:60}]}
    ]});
    const motifKeys=motiveFixture.tracks[1].id;
    await load(motiveFixture.id);
    await switchArrangement(true);
    updateArrangeSelection({start:0,end:2.375,ids:[motifVoice]});
    await $('reviewSong').onclick();
    knowledgeDialog.querySelector('[data-motif-index="0"]').click();
    await until(()=>$('motifWholeSong')&&!$('scoreMotifReview').hidden,'Scoped motif panel did not appear');
    check($('scoreMotifReview').querySelectorAll('.motif-result').length===1,'Scoped motif panel included an outside note');
    $('motifWholeSong').click();
    await until(()=>$('scoreMotifReview').querySelectorAll('.motif-result').length===3,'Whole-song motif view did not show both returns');
    const motifText=$('scoreMotifReview').textContent;
    check(motifText.includes('句尾变化')&&motifText.includes('+2 半音')&&motifText.includes('节奏 ×0.5')&&
      motifText.includes('换词')&&motifText.includes('Dm9')&&motifText.includes('回归处钢琴 2'),
      'The motif view did not preserve the distinct context');
    const auditionEngine=$('renderEngine').value;
    $('renderEngine').value='voicevox';
    $('scoreMotifReview').querySelector('[data-motif-audition="2"]').click();
    await until(()=>reviewPair&&audio.auditionId===reviewPair.id&&audio.auditionVariant===0&&!audio.paused,
      'Motif A did not play from the completed sung mix',25000);
    const motifNativeId=reviewPair.id;
    const motifReceipt=(await api('range_audition',{action:'status',project_id:project.id,
      expected_revision:project.revision})).pair;
    const fromA=await transport();
    check(motifReceipt.saved.job_id===rendered.job_id&&motifReceipt.beat_ranges.map(range=>range.join(',')).join(';')==='0,2.375;8,9.1875'&&
      motifReceipt.frame_ranges[0][1]-motifReceipt.frame_ranges[0][0]===57000&&
      motifReceipt.frame_ranges[1][1]-motifReceipt.frame_ranges[1][0]===28500,
      'Saved comparison used the wrong beats, version or unequal frame lengths');
    $('scoreMotifReview').querySelector('[data-motif-side="B"]').click();
    await until(()=>audio.auditionId===motifNativeId&&audio.auditionVariant===1&&audio.currentTime>=4&&
      audio.currentTime<4.59375&&!audio.paused,'Switching to the shorter sung return left its timeline range');
    const fromB=await transport();
    check(fromB.playing&&fromB.audio_frames>=fromA.audio_frames&&fromB.owner==='user',
      'A/B created a second output stream or handed control away');
    $('scoreMotifReview').querySelector('[data-motif-location="1"]').click();
    await until(()=>!knowledgeDialog.open&&arrangeSelection?.start===4&&arrangeSelection.end===6.625,
      'Motif return did not focus the exact phrase');
    check(cursorBeat===4&&project.revision===motiveFixture.revision,'Reading the return wrote to the project');
    await until(()=>!audio.auditionId,'Closing the review left the saved comparison running');
    let released=false;
    try{await invokeNative('verification_audition',{audition_id:motifNativeId})}catch{released=true}
    if(!released){
      const deadline=performance.now()+4000;
      while(!released&&performance.now()<deadline){
        await new Promise(resolve=>setTimeout(resolve,30));
        try{await invokeNative('verification_audition',{audition_id:motifNativeId})}catch{released=true}
      }
    }
    check(released,'The saved comparison files were not released');
    const mixBeforePhrase=rendered.job_id;
    await $('reviewSong').onclick();
    knowledgeDialog.querySelector('[data-phrase-audition="0"]').click();
    await until(()=>reviewPair&&audio.auditionId===reviewPair.id&&audio.auditionVariant===0&&!audio.paused,
      'The full mix did not start at the selected phrase');
    const phrasePair=reviewPair.id;
    const phraseScope={project_id:project.id,expected_revision:project.revision};
    const phrase=(await api('range_audition',{action:'status',...phraseScope})).pair;
    const phraseA=await transport();
    check(phrase.saved.job_id===mixBeforePhrase&&phrase.saved.outputs[0]===null&&
      phrase.saved.outputs[1][0].kind==='track'&&phrase.saved.outputs[1][0].id===motifVoice&&
      phrase.beat_ranges.every(range=>range.join(',')==='4,6.625')&&
      phrase.gain_db.every(db=>db===0)&&phrase.frame_ranges[0].join(',')===phrase.frame_ranges[1].join(','),
      'The phrase comparison did not reuse the same render and frame range at original levels');
    const phraseControl=$('scorePhraseAudition');
    check(phraseControl.querySelector('[data-motif-side="B"]').textContent==='B 人声轨',
      'The comparison did not identify the output');
    phraseControl.querySelector('[data-motif-side="B"]').click();
    await until(()=>audio.auditionId===phrasePair&&audio.auditionVariant===1&&audio.currentTime>=2&&
      audio.currentTime<3.3125&&!audio.paused,'The vocal output did not switch inside the same phrase');
    const phraseB=await transport();
    check(phraseB.audio_frames>=phraseA.audio_frames&&project.revision===motiveFixture.revision&&
      (await invokeNative('verification_audition',{audition_id:phrasePair})).distinct_pcm,
      'Playing the vocal output rewrote the project or failed to change audible content');
    phraseControl.querySelector('[data-review-levels]').click();
    await until(()=>$('scorePhraseLevels').querySelectorAll('[data-level-window]').length===6,
      'The rendered level windows did not appear');
    const focusWindow=$('scorePhraseLevels').querySelector('[data-level-window="4.5,5"]');
    check(focusWindow?.title.includes('dBFS')&&focusWindow.children.length===2,
      'Level window did not show actual measured signals');
    focusWindow.click();
    await until(()=>reviewPair?.id!==phrasePair&&reviewPair?.id===audio.auditionId&&
      audio.auditionVariant===0&&!audio.paused,'The level window did not start the exact A/B excerpt');
    const windowPair=(await api('range_audition',{action:'status',...phraseScope})).pair;
    check(windowPair.beat_ranges.every(range=>range.join(',')==='4.5,5')&&
      windowPair.saved.job_id===mixBeforePhrase&&focusWindow.getAttribute('aria-pressed')==='true'&&
      !$('scorePhraseLevels').hidden&&project.revision===motiveFixture.revision,
      'Window audition lost its scope, source render or selection');
    report.desktop_render_balance={window_from_real_render:true,exact_window_audition:true};
    knowledgeDialog.close();
    await until(()=>!audio.auditionId,'Closing the review did not stop the phrase comparison');
    const closeDeadline=performance.now()+4000;
    let pending;
    do{
      pending=(await api('range_audition',{action:'status',...phraseScope})).pair;
      if(pending)await new Promise(resolve=>setTimeout(resolve,40));
    }while(pending&&performance.now()<closeDeadline);
    check(!pending,'Closing the review did not release the output comparison');
    report.desktop_phrase_outputs={saved_mix_reused:true,distinct_native_switch:true,no_project_edit:true,released:true};
    updateArrangeSelection({start:4,end:7.25,ids:[motifVoice,motifKeys]});
    await $('reviewSong').onclick();
    const endingPlay=$('knowledgeBody').querySelector('[data-audition-phrase="0"][aria-label^="试听 句尾"]');
    check(endingPlay?.title.includes('5.525–7.25 拍'),'The phrase release did not expose a timed play button');
    endingPlay.click();
    await until(()=>reviewPair&&audio.auditionId===reviewPair.id&&audio.auditionVariant===0&&!audio.paused,
      'The phrase ending did not start a native A/B comparison');
    const endingPair=(await api('range_audition',{action:'status',...phraseScope})).pair;
    check(endingPair.saved.job_id===mixBeforePhrase&&endingPair.frame_ranges.every(range=>range.join(',')==='132600,174000')&&
      endingPair.saved.outputs[1][0].id===motifVoice,
      'The cue comparison did not span the actual backing entrance and use the saved mix');
    const endingProof=await invokeNative('verification_audition',{audition_id:endingPair.id,reference_job_id:mixBeforePhrase,
      hash_window:[165000,168000]});
    check(endingProof.matches_saved_mix&&endingProof.pcm_hashes[0]!==endingProof.pcm_hashes[1],
      'The phrase cue audio did not preserve the full mix or differ after the keys entered');
    const endingControl=$('scorePhraseAudition');
    endingControl.querySelector('[data-motif-side="B"]').click();
    await until(()=>audio.auditionId===endingPair.id&&audio.auditionVariant===1&&!audio.paused,
      'Switching to the vocal output did not retain the current phrase cue');
    endingControl.querySelector('[data-review-levels]').click();
    await until(()=>$('scorePhraseLevels').querySelector('[data-level-window]'),'The phrase cue did not show its own level windows');
    const levelRanges=[...$('scorePhraseLevels').querySelectorAll('[data-level-window]')].map(button=>button.dataset.levelWindow.split(',').map(Number));
    check(Math.abs(levelRanges[0][0]-5.525)<1e-9&&levelRanges.at(-1)[1]===7.25,
      'The level comparison reverted to the full phrase');
    knowledgeDialog.close();
    await until(()=>!reviewPair&&!audio.auditionId,'Closing the review left the short A/B playing');
    report.desktop_phrase_outputs.listening_point_ab=true;
    $('renderEngine').value=auditionEngine;
    report.desktop_saved_ranges={existing_sung_mix:true,unequal_ranges_continuous:true,closed_and_released:true};
    report.desktop_motif={project_id:project.id,scoped_and_explicit_whole_song:true,distinct_context:true,located_return:true};
    let sustained=await api('project_create',{title:'跨音拖腔桌面验证 · 非成品歌曲',bpm:120});
    sustained=await api('composition_design',{project_id:sustained.id,expected_revision:sustained.revision,
      plan:{purpose:'technical_test',brief:'Toggle a touching note into one sustained lyric and audition.'}});
    const sustainTrack=sustained.tracks[0].id;
    sustained=await api('project_edit',{project_id:sustained.id,expected_revision:sustained.revision,
      operations:[{op:'add_notes',track_id:sustainTrack,notes:[
        {id:'vowel_ui_lead',start:.3,duration:1,pitch:65,lyric:'か'},
        {id:'vowel_ui_tail',start:1.3,duration:1.5,pitch:69,lyric:'し'}
      ]}]});
    await load(sustained.id);
    $('rollViewport').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0}));
    chooseGroup(['vowel_ui_lead']);
    check($('vowelContinuation').disabled,'The first note accepted a continuation');
    chooseGroup(['vowel_ui_tail']);
    check(!$('vowelContinuation').disabled,'Two touching vocal notes did not expose continuation');
    $('vowelContinuation').click();
    await until(()=>project.revision===sustained.revision+1&&note()?.vowel_continuation,
      'The continuation did not save to the active note');
    check(displayNote(note(),tr())==='〜'&&$('lyric').disabled&&$('consonantControls').hidden,
      'The continuation still showed independent lyrics or pronunciation controls');
    chooseGroup(['vowel_ui_lead','vowel_ui_tail']);
    $('batchLyrics').click();
    await until(()=>$('lyricsDialog').open,'Could not open lyric placement for a held note');
    check($('lyricsText').value==='か','Lyric placement counted the continuation as a new word');
    $('lyricsDialog').close();
    const tiedRevision=project.revision;
    await $('reviewSong').onclick();
    let syllables=[...knowledgeDialog.querySelectorAll('.score-syllable')];
    check(syllables.map(button=>button.textContent).join(',')==='か〜',
      'Review displayed a dormant lyric for the trailing note');
    syllables[0].click();
    await until(()=>!knowledgeDialog.open&&workspaceFocus==='editor'&&picked().length===2,
      'Clicking the syllable did not select both held pitches');
    check(cursorBeat===.3&&picked().map(n=>n.id).join(',')==='vowel_ui_lead,vowel_ui_tail',
      'The lyric led to the wrong positions or notes');
    await switchArrangement(true);
    updateArrangeSelection({start:1.45,end:2.8,ids:[sustainTrack]});
    await $('reviewSong').onclick();
    syllables=[...knowledgeDialog.querySelectorAll('.score-syllable')];
    check(syllables.map(button=>button.textContent).join(',')==='〜',
      'A clipped lyric exposed a head outside the score range');
    syllables[0].click();
    await until(()=>!knowledgeDialog.open&&workspaceFocus==='editor'&&picked().length===1,
      'Clicking the clipped hold did not return to the piano roll');
    check(cursorBeat===1.45&&note().id==='vowel_ui_tail'&&project.revision===tiedRevision,
      'Scoped lyric location selected outside its range or changed the score');
    report.desktop_syllable_review={grouped_and_located:true,clipped_scope:true};
    const previousPreviewEngine=$('renderEngine').value;
    try{
      $('renderEngine').value='voicevox';
      await render(true);
      await until(()=>!audio.paused&&audio.liveEngine==='voicevox','Could not preview the sustained vowel');
      $('stop').click();
    }finally{$('renderEngine').value=previousPreviewEngine}
    chooseGroup(['vowel_ui_tail']);
    $('vowelContinuation').click();
    await until(()=>project.revision===sustained.revision+2&&!note()?.vowel_continuation,
      'The continuation could not be removed');
    check(note().lyric==='し'&&!$('lyric').disabled,'Removing continuation lost the stored lyric');
    report.desktop_vowel_continuation=true;

    const agentCall=(name,args)=>invokeNative('verification_agent_call',{name,arguments:args});
    await load(motiveFixture.id);
    const scopedAction={project_id:project.id,expected_revision:project.revision};
    const trackForIntent=project.tracks[0];
    const finalNote=trackForIntent.notes.at(-1);
    const announce=(scope,reason)=>agentCall('agent_focus',{...scope,region:'velocity',reason,
      track_id:trackForIntent.id,note_ids:[finalNote.id]});
    const oldIntent=await announce(scopedAction,'检查这句开头的余量');
    const editReason='收轻副歌最后一个音，让下一句留出气口';
    const editIntent=await announce(scopedAction,editReason);
    const oneNoteChange={op:'update_note',track_id:trackForIntent.id,note_id:finalNote.id,
      values:{velocity:67}};
    const editedForIntent=await agentCall('project_edit',{...scopedAction,intent_id:editIntent.intent_id,
      operations:[oneNoteChange]});
    await agentCall('project_read',{project_id:project.id});
    await until(()=>$('agentAction').textContent===editReason&&$('agentOutcome').textContent==='已保存 1 项修改'&&
      $('agentPhase').textContent==='完成','The saved edit lost its declared intent');
    check(editedForIntent.tracks[0].notes.at(-1).velocity===67,'The contextual edit did not reach the announced note');
    let staleContextRejected=false;
    try{await agentCall('project_edit',{...scopedAction,intent_id:oldIntent.intent_id,
      operations:[oneNoteChange]})}catch(error){staleContextRejected=error.message.includes('REVISION_CONFLICT')}
    const {sequence:staleSequence}=await api('agent_events',{after:0});
    await until(()=>eventSequence>=staleSequence,'The completed action was not observed');
    check(staleContextRejected&&$('agentAction').textContent===editReason&&$('agentPhase').textContent==='完成',
      'A delayed older intent replaced the newer action');
    const renderScope={...scopedAction,expected_revision:editedForIntent.revision};
    const renderReason='导出副歌新尾音供复听';
    const renderIntent=await agentCall('agent_focus',{...renderScope,region:'transport',reason:renderReason});
    const contextualRender=await agentCall('render_start',{...renderScope,engine:'guide',
      intent_id:renderIntent.intent_id});
    await until(()=>$('agentAction').textContent===renderReason&&$('agentOutcome').textContent==='试听音频已渲染'&&
      $('agentPhase').textContent==='完成','The background render lost the declared task',18000);
    check((await api('render_status',{job_id:contextualRender.job_id})).status==='complete',
      'A render result was shown before the job completed');
    report.desktop_bound_agent_intent=true;

    const liveApi=api;
    let finishSlowRead;
    let slowReadStarted=false;
    api=(name,args)=>!slowReadStarted&&name==='project_read'&&args.project_id===motiveFixture.id
      ? new Promise(resolve=>{slowReadStarted=true;finishSlowRead=()=>resolve(liveApi(name,args))})
      : liveApi(name,args);
    try{
      const slowOpening=load(motiveFixture.id);
      await until(()=>slowReadStarted,'The first project read was not held');
      await load(pid);
      finishSlowRead();
      await slowOpening;
      check(project.id===pid&&localStorage.getItem(lastProjectPreference)===pid,
        'An older request changed the visible or remembered project');
      localStorage.setItem(lastProjectPreference,motiveFixture.id);
      try{
        await restoreWorkspace();
        check(project.id===pid,'A late session restore changed the open project');
      }finally{localStorage.setItem(lastProjectPreference,pid)}
      report.desktop_workspace_newest_open_wins=true;
    }finally{api=liveApi}

    const projectRow=()=>$('projects').querySelector(`[data-project="${CSS.escape(pid)}"]`);
    const existingProjectRow=projectRow();
    check(existingProjectRow,'The active project did not have a catalog row');
    let releaseCatalog;
    api=(name,args)=>name==='project_list'&&!releaseCatalog
      ? new Promise(resolve=>{releaseCatalog=()=>resolve(liveApi(name,args))})
      : liveApi(name,args);
    try{
      project=null;
      const restoration=restoreWorkspace();
      await until(()=>releaseCatalog&&project?.id===pid,'Restoring the project waited for the catalog');
      releaseCatalog();
      releaseCatalog=()=>{};
      await restoration;
      check(projectRow()===existingProjectRow,'Refreshing the catalog replaced the active project row');
      report.desktop_project_catalog={direct_open_during_listing:true,rows_reused:true};
    }finally{releaseCatalog?.();api=liveApi}

    {
      $('openProjects').click();
      await until(()=>drawer.open&&document.activeElement===$('projectSearch'),
        'The project drawer did not focus its search');
      const projectRows=()=>$('projects').querySelectorAll('.project-item');
      const findProject=value=>{
        $('projectSearch').value=value;
        $('projectSearch').dispatchEvent(new Event('input',{bubbles:true}));
      };
      findProject(pid);
      await until(()=>$('projectListStatus').textContent==='没有匹配的工程'&&!$('projects').inert,
        'Workspace search included an explicitly marked test score');
      check(projectRows().length===0,'Search pinned a project outside the filter');
      $('projectTests').click();
      await until(()=>!$('projects').inert&&projectRows().length===1&&projectRow(),
        'Technical view could not find the same project by ID');
      let releaseObsolete;
      api=(name,args)=>name==='project_list'&&args?.query==='obsolete catalog result'
        ? new Promise(resolve=>{releaseObsolete=resolve})
        : liveApi(name,args);
      try{
        findProject('obsolete catalog result');
        await until(()=>releaseObsolete,'The old search request did not begin');
        findProject(pid);
        await until(()=>!$('projects').inert&&projectRows().length===1&&projectRow(),
          'The new search waited for the obsolete request');
        const retained=projectRow();
        releaseObsolete([{id:'obsolete',title:'Obsolete result'}]);
        await new Promise(resolve=>setTimeout(resolve,50));
        check(projectRow()===retained&&projectRows().length===1,'A stale search result replaced the current page');
      }finally{releaseObsolete?.([]);api=liveApi}
      findProject('');
      await until(()=>!$('projects').inert&&$('projectPage').textContent==='1',
        'Clearing the search did not reset the page');
      const firstPageIds=[...projectRows()].map(row=>row.dataset.project);
      check(firstPageIds.length<=projectPageSize,'The project drawer rendered an unbounded result');
      if(!$('projectNext').disabled){
        $('projectNext').click();
        await until(()=>!$('projects').inert&&$('projectPage').textContent==='2','The next project page did not load');
        check([...projectRows()].every(row=>!firstPageIds.includes(row.dataset.project)),
          'The next page duplicated an ID from the first page');
        $('projectPrev').click();
        await until(()=>!$('projects').inert&&$('projectPage').textContent==='1','The previous page did not load');
      }
      $('projectTests').click();
      await until(()=>!$('projects').inert&&$('projectTests').getAttribute('aria-pressed')==='false',
        'The drawer did not return to the workspace');
      drawer.close();
      await until(()=>!drawer.open,'The project drawer did not close');
      let closedRequests=0;
      api=(name,args)=>{
        if(name==='project_list')closedRequests++;
        return liveApi(name,args);
      };
      try{
        await refreshList();
        check(closedRequests===0,'A closed project drawer still scanned the catalog');
      }finally{api=liveApi}
      report.desktop_project_catalog.search_scoped=true;
      report.desktop_project_catalog.stale_search_ignored=true;
      report.desktop_project_catalog.bounded_pages=true;
      report.desktop_project_catalog.closed_list_idle=true;
    }
    let codecFixture=await api('project_create',{title:'桌面音频导入验证 · 非成品歌曲',bpm:120});
    codecFixture=await api('composition_design',{project_id:codecFixture.id,
      expected_revision:codecFixture.revision,plan:{purpose:'technical_test',brief:'Import compressed sources using both native file menus.'}});
    await load(codecFixture.id);
    const findFile=name=>[...$('studioFileEntries').querySelectorAll('[data-path]')]
      .find(button=>button.dataset.path.endsWith('\\'+name));
    await until(()=>!document.querySelector('dialog[open]'),'The previous dialog did not close');
    cursorBeat=.625;
    key(document.body,'o',{ctrlKey:true});
    await inAppFolder();
    const selectFlac=findFile('格式导入验证_48k.FLAC');
    check(selectFlac&&findFile('格式导入验证_44k.MP3'),'The open menu did not list both compressed formats');
    selectFlac.click();$('studioFileApply').click();
    await until(()=>project.id===codecFixture.id&&project.revision===codecFixture.revision+1,
      'The FLAC import from Ctrl+O did not commit');
    const flacTrack=project.tracks.at(-1);
    check(flacTrack.audio?.seconds===12336/48000&&flacTrack.audio.start===.625&&trackId===flacTrack.id,
      'FLAC did not appear on a new track at the edit cursor');
    if(!arrangementVisible)await switchArrangement(true);
    await until(()=>!$('clipAppend').hidden&&!filePanel.open,'The audio track did not show its add audio button');
    cursorBeat=2.25;
    $('clipAppend').click();
    await inAppFolder();
    const selectMp3=findFile('格式导入验证_44k.MP3');
    check(selectMp3&&findFile('格式导入验证_48k.FLAC')&&
      !findFile(localExport),'The add audio dialog did not filter to audio formats');
    selectMp3.click();$('studioFileApply').click();
    await until(()=>project.revision===codecFixture.revision+2,'MP3 did not append to the current audio track');
    const importedClips=project.tracks.at(-1).clips;
    check(project.tracks.at(-1).id===flacTrack.id&&importedClips.length===2&&
      importedClips[1].start===2.25&&importedClips[1].frames===Math.round(18963*48000/44100),
      'The appended MP3 has the wrong track, timing or gapless length');
    const mp3Shape=await api('audio_waveform',{project_id:project.id,track_id:flacTrack.id,clip_id:importedClips[1].id});
    check(mp3Shape.peaks.some(peak=>peak[0]<-.1&&peak[1]>.1),'Imported MP3 waveform is missing');
    report.desktop_audio_formats={flac_at_cursor_from_open:true,mp3_appended_to_same_track:true,
      picker_filters_extensions:true,decoded_waveform:true};

    let takeFixture=await api('project_create',{title:'录音挑选验证 · 非成品歌曲',bpm:120});
    const takeBinding=()=>({project_id:takeFixture.id,expected_revision:takeFixture.revision});
    takeFixture=await api('composition_design',{...takeBinding(),
      plan:{purpose:'technical_test',brief:'Compare and copy the same range of two dry takes.'}});
    for(const [name,samples] of [['首遍',payload],['次遍',attackPayload]]){
      takeFixture=await api('audio_import',{...takeBinding(),name,filename:name+'.wav',data_base64:btoa(samples)});
    }
    const takeSources=takeFixture.tracks.slice(-2);
    await load(takeFixture.id);
    if(!arrangementVisible)await switchArrangement(true);
    await until(()=>!document.querySelector('dialog[open]'),'The previous dialog did not close');
    const agentTakes=await agentCall('range_audition',{...takeBinding(),action:'prepare',level_match:false,
      beat_ranges:[[.5,1.5],[.5,1.5]],recordings:takeSources.map(track=>({track_id:track.id,clip_id:'legacy'}))});
    try{
      const transportBefore=await agentCall('desktop_transport',{action:'status'});
      const started=await agentCall('desktop_transport',{...takeBinding(),action:'audition_side',
        ticket:transportBefore.ticket,audition_id:agentTakes.id,variant:'B',reason:'复听选中的次遍录音'});
      check(started.playing&&started.owner==='agent','MCP could not play the recording comparison');
    }finally{
      const transportAfter=await agentCall('desktop_transport',{action:'status'});
      if(transportAfter.owner==='agent')await agentCall('desktop_transport',{action:'stop',ticket:transportAfter.ticket});
      await agentCall('range_audition',{...takeBinding(),action:'release',audition_id:agentTakes.id});
    }
    const pickTakes=()=>{
      setClipGroup(takeSources.map(track=>({track,clip:trackClips(track)[0]})));
      updateArrangeSelection({start:.5,end:1.5,ids:takeSources.map(track=>track.id)});
    };
    pickTakes();
    check(!$('groupTakes').hidden,'The source pair did not enable take comparison');
    $('groupTakes').click();
    await until(()=>takeDialog.open,'Take comparison did not open');
    check($('takeStart').valueAsNumber===.5&&$('takeEnd').valueAsNumber===1.5,
      'Take comparison ignored the box range');
    $('takeA').click();
    await until(()=>!audio.paused&&audio.auditionVariant===0&&!!takeReview?.pair,
      'A did not play from the source clip');
    const takePair=takeReview.pair;
    check(takePair.frame_ranges.every(range=>range[0]===12000&&range[1]===36000),
      'Take comparison did not use the same sample bounds');
    const dryTakes=await invokeNative('verification_audition',{audition_id:takePair.id});
    check(dryTakes.distinct_pcm,'The two takes produced the same playback data');
    await until(()=>$('takeA').disabled===false,'Take comparison did not finish preparing A');
    $('takeExport').click();
    await inAppFolder();
    const takeZipName=`Studio_AB_${Date.now()}.zip`;
    check($('studioFileName').value===`试听对照_r${takeFixture.revision}.zip`,'Take export did not suggest a ZIP');
    $('studioFileName').value=takeZipName;
    await until(()=>!$('studioFileApply').disabled,'Take export destination did not become available');
    $('studioFileApply').click();
    await until(()=>!filePanel.open&&!$('takeExport').disabled,'The take comparison ZIP did not save');
    const takeZips=await invokeNative('file_browse',{directory:fixtureFolder,extension:'zip',query:takeZipName});
    check(takeZips.entries.length===1&&takeZips.entries[0].bytes>288000,'Take ZIP is missing its audio');
    report.desktop_take_delivery=takeZips.entries[0].path;
    $('takeB').click();
    await until(()=>!audio.paused&&audio.auditionVariant===1&&audio.auditionId===takePair.id,
      'B did not switch within the same comparison');
    check(audio.currentTime>=.25&&audio.currentTime<.75,'B switch left the selected region');
    await until(()=>$('takeUseB').disabled===false,'Take comparison did not finish switching');
    $('takeUseB').click();
    await until(()=>!takeDialog.open&&project.revision===takeFixture.revision+1,'The selected take was not copied');
    const pickedTake=project.tracks.at(-1).clips[0];
    check(trackId===project.tracks.at(-1).id&&pickedTake.asset===takeSources[1].audio.asset&&
      pickedTake.start===.5&&pickedTake.offset===12000&&pickedTake.frames===24000,
      'The comp did not reference the chosen take with exact boundaries');
    check(JSON.stringify(project.tracks.slice(0,-1))===JSON.stringify(takeFixture.tracks),
      'Copying the take changed its original track');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(takeFixture.tracks),'One Undo did not remove the new comp track');
    report.desktop_take_review={box_range:true,agent_can_play:true,same_pair_switch:true,source_pcm:true,exact_copy:true,one_undo:true};

    const streamingFixture=await invokeNative('verification_long_mix',{});
    const longMix=(await api('render_status',{job_id:streamingFixture.job_id})).result;
    check(longMix.seconds>60,'Long playback test requires the existing full song mix');
    await load(streamingFixture.project_id);
    check(project.revision===longMix.revision,'Long mix was not from the visible project revision');
    const requestStart=performance.now();
    const cacheFrameLimit=5*(24576+128);
    const streamed=await invokeNative('transport',{command:'play',url:longMix.mix_url,seconds:18.3,
      loop:{start:18.3,end:19.43}});
    const startMs=performance.now()-requestStart;
    check(streamed.streaming&&streamed.playing&&streamed.buffered_frames>0&&streamed.buffered_frames<=cacheFrameLimit&&
      streamed.sample_rate_conversion===converter&&
      Math.abs(streamed.duration-longMix.seconds)<1e-6,'Long mix did not use a bounded, prefilled cache');
    async function awaitPaged(predicate,message,attempts=160){
      for(let i=0;i<attempts;i++){
        const state=await transport();
        if(predicate(state))return state;
        await new Promise(resolve=>setTimeout(resolve,18));
      }
      throw new Error(message);
    }
    const pagedLoop=await awaitPaged(state=>state.loop_wraps>0&&state.click_pulses===0&&state.peak.some(x=>x>.005),
      'Paged playback did not read across the loop boundary');
    check(pagedLoop.buffer_wait_frames===0&&pagedLoop.seconds>=18.3&&pagedLoop.seconds<19.43,
      'Real output underrun or incorrect loop position before any seek');
    await invokeNative('transport',{command:'loop',loop:null});
    await invokeNative('transport',{command:'seek',seconds:51});
    await invokeNative('transport',{command:'seek',seconds:55});
    const jumped=await awaitPaged(state=>state.playing&&state.seconds>55.04&&state.seconds<55.8&&
      !state.buffering&&state.peak.some(x=>x>.003),'Paged playback did not resume at the latest far seek');
    check(!jumped.stream_error&&jumped.buffered_frames<=cacheFrameLimit,'Seek released the cache bound or stopped playback');
    await invokeNative('transport',{command:'stop'});
    const bounded=(await api('timeline_query',{project_id:project.id,seconds:[53,53.32]})).second_positions.map(x=>x.quarter_beat);
    const permission=await agentCall('desktop_transport',{action:'status'});
    const pagedPreview=await agentCall('desktop_transport',{action:'play',project_id:project.id,
      expected_revision:project.revision,ticket:permission.ticket,job_id:streamingFixture.job_id,
      beat_range:bounded,reason:'在长混音后半段复听',repeat:false});
    check(pagedPreview.streaming&&pagedPreview.owner==='agent'&&pagedPreview.playing,
      'The Agent did not open the long mix as a bounded preview');
    const endPreview=await awaitPaged(state=>state.ended&&!state.playing,'Paged Agent preview did not stop at its endpoint');
    check(Math.abs(endPreview.seconds-53.32)<1/48000&&endPreview.buffered_frames<=cacheFrameLimit,
      'Paged one-shot advanced beyond the requested sample boundary');
    const agentDone=await agentCall('desktop_transport',{action:'status'});
    if(agentDone.owner==='agent')await agentCall('desktop_transport',{action:'stop',ticket:agentDone.ticket});
    report.desktop_streaming={existing_song:true,loop_without_wait:true,latest_seek:true,agent_exact_stop:true,
      duration_seconds:streamed.duration,initial_open_ms:Math.round(startMs*100)/100,
      cache_frames:Math.max(streamed.buffered_frames,pagedLoop.buffered_frames,jumped.buffered_frames),
      full_frames:Math.round(streamed.duration*48000),seek_wait_device_frames:jumped.buffer_wait_frames,
      output_rate:streamed.output_rate};

    const multitrack=await invokeNative('verification_live_stream',{});
    check(multitrack.version===host.version&&multitrack.prepared_tracks===16&&multitrack.full_track_bytes>384*1024*1024,
      'The real live-monitor fixture does not exceed the old dry buffer ceiling');
    await load(multitrack.project_id);
    const longRender=(await api('render_status',{job_id:multitrack.job_id})).result;
    const liveBind={project_id:project.id,revision:project.revision,job_id:multitrack.job_id};
    await invokeNative('transport',{command:'play',url:longRender.mix_url,seconds:18.3,loop:{start:18.3,end:19.43}});
    const readyStart=performance.now();
    const largeMonitor=await invokeNative('transport',{command:'monitor_mix',...liveBind,draft:{tracks:[]}});
    const monitorOpenMs=performance.now()-readyStart;
    check(largeMonitor.live_mix&&largeMonitor.live_streaming&&largeMonitor.monitor_track_ids.length===16,
      'Dry input pages were not loaded for all tracks');
    const steadyStart=performance.now();
    const liveLoop=await awaitPaged(state=>state.live_frames>32000&&state.loop_wraps>0&&state.peak.some(x=>x>.004),
      'The dry pages did not sustain a complete real output loop');
    const liveSpeed=(liveLoop.live_frames-largeMonitor.live_frames)/liveLoop.output_rate/
      ((performance.now()-steadyStart)/1000);
    check(liveLoop.buffer_wait_frames===largeMonitor.buffer_wait_frames&&liveLoop.live_buffered_frames<=16*cacheFrameLimit&&
      liveLoop.sample_rate_conversion===converter&&liveSpeed>.8,
      'Steady live mixing dropped frames or exceeded the dry cache cap');
    await invokeNative('transport',{command:'loop',loop:null});
    await invokeNative('transport',{command:'seek',seconds:51});
    await invokeNative('transport',{command:'seek',seconds:55});
    const movedMonitor=await awaitPaged(state=>state.live_streaming&&state.seconds>55.04&&state.seconds<55.9&&
      !state.buffering&&state.peak.some(x=>x>.004),'Paged mixer did not advance from the last seek');
    const updatedMonitor=await invokeNative('transport',{command:'monitor_mix',...liveBind,draft:{tracks:[
      {track_id:multitrack.source_track,gain_db:-12,pan:.35}
    ]}});
    check(updatedMonitor.live_streaming&&updatedMonitor.mix_changes>movedMonitor.mix_changes&&
      updatedMonitor.url===movedMonitor.url,'A fader or pan change reopened the source');
    const agentMonitor=await agentCall('desktop_transport',{action:'status'});
    check(agentMonitor.live_streaming&&agentMonitor.live_buffered_frames>0&&
      agentMonitor.live_buffered_frames<=16*cacheFrameLimit&&agentMonitor.sample_rate_conversion===converter,
      'MCP did not report the actual dry page count and converter');
    const nextMonitor=await agentCall('project_edit',{project_id:project.id,expected_revision:project.revision,
      operations:[{op:'update_track',track_id:multitrack.muted_track,values:{mute:false,gain_db:-18}}]});
    accept(nextMonitor,true);
    const advancedMonitor=await awaitPaged(state=>state.revision===nextMonitor.revision&&state.live_streaming&&
      state.playing&&state.live_frames>movedMonitor.live_frames,
      'A revision-bound Agent unmute did not reuse the already prepared dry input');
    check(advancedMonitor.url===largeMonitor.url&&!advancedMonitor.stream_error,
      'Applying an Agent mix change stopped live playback');
    await invokeNative('transport',{command:'loop',loop:{start:55.05,end:55.65}});
    const beforeSource=await awaitPaged(state=>state.seconds>=55.05&&state.seconds<55.65&&
      state.track_peaks[0]?.some(value=>value>.025),'The long source had no measurable signal');
    const refreshClip={project_id:project.id,expected_revision:project.revision,
      track_id:multitrack.source_track,clip_id:'legacy',action:{kind:'set',gain_db:-48}};
    const clipDraft=await agentCall('audio_clip_edit',refreshClip);
    const revisedClip=(await agentCall('audio_clip_edit',{...refreshClip,preview_id:clipDraft.preview_id})).project;
    const sourceStarted=performance.now();
    accept(revisedClip,true);
    const refreshedMonitor=await awaitPaged(state=>state.monitor_revision===revisedClip.revision&&
      !state.monitor_refreshing&&state.playing&&state.live_streaming&&
      state.track_peaks[0]?.every(value=>value<.008),'Updated clip input did not reach the WASAPI mixer');
    check(refreshedMonitor.audio_frames>beforeSource.audio_frames&&refreshedMonitor.url===beforeSource.url&&
      refreshedMonitor.seconds>=55.05&&refreshedMonitor.seconds<55.65&&
      refreshedMonitor.callbacks>beforeSource.callbacks&&!refreshedMonitor.stream_error,
      'Refreshing the dry source restarted the clock or lost its loop');
    const refreshedJobs=await api('render_jobs',{project_id:project.id,expected_revision:project.revision,engine:'guide'});
    check(refreshedJobs.jobs.length===0,'A monitor update produced an export job');
    await invokeNative('transport',{command:'stop'});
    report.desktop_live_stream={uses_paged_dry:true,loop_without_wait:true,last_seek:true,
      fader_without_reload:true,agent_unmuted_prepared_track:true,
      prepared_tracks:16,open_ms:Math.round(monitorOpenMs*100)/100,full_track_bytes:multitrack.full_track_bytes,
      output_rate:liveLoop.output_rate,observed_clock_ratio:Math.round(liveSpeed*1000)/1000,
      resident_dry_bytes:Math.max(largeMonitor.live_buffered_frames,liveLoop.live_buffered_frames,
        movedMonitor.live_buffered_frames,advancedMonitor.live_buffered_frames)*8};
    report.desktop_monitor_refresh={paged_clip_kept_loop:true,no_export_job:true,
      paged_refresh_ms:Math.round(performance.now()-sourceStarted)};
    let refreshVoice=await api('project_create',{title:'歌声音轨接续验证 · 非成品歌曲',bpm:120});
    const refreshTrack=refreshVoice.tracks[0].id;
    refreshVoice=await api('composition_design',{project_id:refreshVoice.id,expected_revision:refreshVoice.revision,
      plan:{purpose:'technical_test',brief:'Verify native monitor refresh on a changed vocal note'}});
    refreshVoice=await api('project_edit',{project_id:refreshVoice.id,expected_revision:refreshVoice.revision,
      operations:[{op:'add_track',track_ref:'monitorKeys',values:{name:'分页验证键盘',instrument:{program:0},mute:true}},
        {op:'add_notes',track_ref:'monitorKeys',notes:[{start:1,duration:1,pitch:60}]},
        {op:'add_notes',track_id:refreshTrack,notes:[
        {id:'refresh_head',start:.25,duration:1,pitch:62,lyric:'あ'},
        {id:'refresh_tail',start:1.25,duration:1,pitch:64,lyric:'あ',vowel_continuation:true},
        {id:'refresh_later',start:16,duration:1,pitch:67,lyric:'か'},
        {id:'refresh_end',start:49,duration:1,pitch:66,lyric:'え'}
      ]}]});
    await load(refreshVoice.id);
    $('renderEngine').value='voicevox';
    const voiceFullStart=performance.now();
    await render(true);
    const voicePlayback=await awaitPaged(state=>state.playing&&state.live_mix&&state.live_streaming&&state.duration>24&&
      state.monitor_track_ids.length===2&&
      state.monitor_revision===refreshVoice.revision,'Could not start the real vocal monitor');
    const voiceFullMs=Math.round(performance.now()-voiceFullStart);
    await invokeNative('transport',{command:'loop',loop:{start:.25,end:1}});
    const editRefreshPitch=async (pitch,noteId='refresh_tail')=>{
      const revision=await agentCall('project_edit',{project_id:project.id,expected_revision:project.revision,
        operations:[{op:'update_note',track_id:refreshTrack,note_id:noteId,values:{pitch}}]});
      accept(revision,true);
      return revision.revision;
    };
    const localStarted=performance.now();
    await editRefreshPitch(63);
    const finalVoice=await editRefreshPitch(67);
    const voiceUpdated=await awaitPaged(state=>state.playing&&state.live_mix&&state.monitor_revision===finalVoice&&
      !state.monitor_refreshing&&state.live_frames>voicePlayback.live_frames,'The latest vocal edit did not reach the live stream',600);
    check(voiceUpdated.url===voicePlayback.url&&voiceUpdated.seconds>=.25&&voiceUpdated.seconds<1&&
      voiceUpdated.audio_frames>voicePlayback.audio_frames,'Preparing real singing restarted monitor playback');
    const localMs=Math.round(performance.now()-localStarted);
    check(voiceUpdated.monitor_local_frames>48000&&voiceUpdated.monitor_local_frames<80000,
      'Changed continuation was not joined to its complete syllable in a local PCM update');
    await invokeNative('transport',{command:'pause'});
    const pausedVoice=await editRefreshPitch(65,'refresh_later');
    const pausedUpdated=await awaitPaged(state=>!state.playing&&state.monitor_revision===pausedVoice&&
      !state.monitor_refreshing,'Updating a paused vocal monitor resumed playback or lost the new source',600);
    check(pausedUpdated.url===voicePlayback.url,'Preparing a paused edit loaded a different mix');
    check(pausedUpdated.monitor_local_frames>48000&&pausedUpdated.monitor_local_frames<80000,
      'A separately edited phrase did not patch the prior monitoring source');
    const voiceJobs=await api('render_jobs',{project_id:project.id,expected_revision:pausedVoice,engine:'voicevox'});
    check(!voiceJobs.jobs.length,'A local voice update was exposed as an exported mix');
    report.desktop_monitor_refresh.voice_latest_kept_clock=true;
    report.desktop_monitor_refresh.paused_edit_stays_paused=true;
    report.desktop_local_voice_monitor={complete_syllable:true,second_phrase_stays_local:true,monitor_only:true,
      track_seconds:voicePlayback.duration,first_full_ms:voiceFullMs,latest_patch_ms:localMs,
      latest_patch_seconds:voiceUpdated.monitor_local_frames/48000};
    let fitScore=await api('project_create',{title:'歌词贴旋律验证 · 非成品歌曲',bpm:120});
    fitScore=await api('composition_design',{project_id:fitScore.id,expected_revision:fitScore.revision,
      plan:{purpose:'technical_test',brief:'Verify phrase text and actual note readings'}});
    fitScore=await api('project_edit',{project_id:fitScore.id,expected_revision:fitScore.revision,
      operations:[{op:'add_notes',track_id:fitScore.tracks[0].id,notes:[
        {id:'fit_outside',start:0,duration:.2,pitch:60,lyric:'お'},
        {id:'fit_a',start:.5,duration:.5,pitch:65,lyric:'さ'},
        {id:'fit_link',start:1,duration:.16,pitch:67,lyric:'ら'},
        {id:'fit_b',start:1.16,duration:.48,pitch:69,lyric:'く'},
        {id:'fit_c',start:2.3,duration:.55,pitch:64,lyric:'な'}
      ]}]});
    await load(fitScore.id);
    workspaceFocus='editor';
    const fitIds=['fit_a','fit_link','fit_b','fit_c'];
    chooseGroup(fitIds);
    await $('batchLyrics').onclick();
    check($('lyricsDialog').open,'Could not open lyrics for the selected phrase');
    $('lyricsMode').value='fit';
    $('lyricsSource').value='赤絵';
    $('lyricsText').value='あ か え';
    await $('lyricsPreview').onclick();
    check($('lyricsAlignment').children.length===3&&$('lyricsAlignment').textContent.includes('ア〜')&&
      $('lyricsAlignment').textContent.includes('2 音')&&!$('applyLyrics').disabled&&project.revision===fitScore.revision,
      'The lyric fit did not show the complete preview without saving');
    await $('applyLyrics').onclick();
    await until(()=>!$('lyricsDialog').open&&project.revision===fitScore.revision+1,
      'Lyrics save or the dialog exit did not finish');
    check(tr().notes.find(note=>note.id==='fit_link').vowel_continuation&&
      tr().notes.find(note=>note.id==='fit_outside').lyric==='お','Lyrics did not apply inside the intended selection');
    check(tr().lyric_lines?.[0]?.text==='赤絵'&&tr().lyric_lines[0].note_ids.length===4,
      'The phrase lost its written lyric');
    const fitRevision=project.revision;
    updateArrangeSelection({start:.5,end:2.85,ids:[tr().id]});
    $('arrangeCopy').click();
    $('arrangeTarget').value=4;
    await $('arrangePreview').onclick();
    const lyricChangeList=$('arrangePitchRanges');
    check(!lyricChangeList.hidden&&lyricChangeList.textContent.includes('赤絵')&&
      lyricChangeList.textContent.includes('复制原词'),'Whole-phrase copy did not show the original lyric');
    $('arrangeFrom').value=2.3;
    $('arrangeFrom').dispatchEvent(new Event('input',{bubbles:true}));
    check(lyricChangeList.hidden&&$('arrangeApply').disabled,'Changing the range left the old result active');
    await $('arrangePreview').onclick();
    check(!lyricChangeList.hidden&&lyricChangeList.textContent.includes('复制 1/4 音')&&
      !lyricChangeList.textContent.includes('赤絵')&&project.revision===fitRevision,
      'Partial copy did not explain the affected notes within the range');
    rangeDialog.close();
    await until(()=>!rangeDialog.open,'The arrangement preview did not finish closing');
    workspaceFocus='editor';
    await $('reviewSong').onclick();
    check(knowledgeDialog.open&&$('knowledgeBody').textContent.includes('赤絵')&&
      $('knowledgeBody').querySelector('[data-written-line]'),'The score review did not show the written lyric');
    $('knowledgeBody').querySelector('[data-written-line]').click();
    await until(()=>!knowledgeDialog.open&&picked().length===4,'Written lyric did not locate the exact original notes');
    await $('batchLyrics').onclick();
    check($('lyricsDialog').open&&$('lyricsSource').value==='赤絵','Reopening the phrase lost its written lyric');
    $('lyricsDialog').close();
    await until(()=>!$('lyricsDialog').open,'The lyrics dialog did not finish closing');
    chooseGroup(fitIds);
    check(copyPhrase(),'Could not copy complete phrase');
    pastePhrase(4);
    await until(()=>tr().lyric_lines?.length===2,'Pasting the complete phrase dropped the original lyric');
    check(tr().lyric_lines[1].text==='赤絵'&&tr().lyric_lines[1].note_ids.every(id=>!fitIds.includes(id)),
      'Pasting a phrase did not assign fresh lyric anchors');
    await $('undo').onclick();
    await $('undo').onclick();
    check(JSON.stringify(project.tracks)===JSON.stringify(fitScore.tracks),'One undo did not restore the phrase');
    report.desktop_lyric_fit={preview_groups:3,rest_preserved:true,one_undo:true};
    report.desktop_lyric_lines={saved_located_reopened:true,roll_copies_complete_line:true,undo_removes_source:true,
      arrangement_preview_tracks_scope:true};
    fitScore=await api('audio_import',{project_id:project.id,expected_revision:project.revision,
      filename:'整段范围.wav',name:'编辑测试音频',data_base64:btoa(payload),start:.25});
    accept(fitScore);
    const mixedIds=fitScore.tracks.map(track=>track.id);
    const box={start:.5,end:2,ids:mixedIds};
    workspaceFocus='editor';
    $('arrangeRange').click();
    $('arrangeFrom').value=box.start;
    $('arrangeTo').value=box.end;
    $('arrangeTrackFields').querySelectorAll('input').forEach(input=>input.checked=true);
    await $('arrangeApply').onclick();
    await until(()=>!rangeDialog.open,'Range selection did not close: '+$('arrangePreviewText').textContent);
    check(workspaceFocus==='arrangement'&&currentStudioSelection().clip_refs.length===1,
      'Explicit range selection did not capture the audio clip');
    const boxBinding={project_id:project.id,expected_revision:project.revision};
    const readCurrentBox=async (start,end)=>{
      for(let i=0;i<40;i++){
        const item=await agentCall('arrangement_query',{...boxBinding,from_ui:true}).catch(()=>null);
        if(item?.beat_range[0]===start&&item.beat_range[1]===end)return item;
        await new Promise(resolve=>setTimeout(resolve,40));
      }
      throw new Error('The desktop did not publish its arrangement box');
    };
    const capturedBox=await readCurrentBox(.5,2);
    const [boxVoice,boxAudio]=capturedBox.tracks;
    check(boxVoice.note_count===3&&boxVoice.notes.every(note=>note.note_id!=='fit_outside')&&
      boxAudio.clip_count===1&&boxAudio.clips[0].partial&&boxAudio.clips[0].timing==='fixed'&&
      boxAudio.clips[0].beat_range.join(',')==='0.5,2','The inventory did not match the selected audio and score');
    const boundCopy={action:'copy',track_ids:mixedIds,source:[.5,2],target_start:4,slice_audio:true};
    const previewBox=await agentCall('arrangement_edit',{...boxBinding,...boundCopy,box_id:capturedBox.box_id});
    const emptyBox={start:4,end:5.5,ids:mixedIds};
    updateArrangeSelection(emptyBox);selectClipsInRange(emptyBox);
    const latestBox=await readCurrentBox(4,5.5);
    check(latestBox.box_id!==capturedBox.box_id&&latestBox.tracks.length===2&&
      latestBox.tracks.every(track=>!track.note_count&&!track.clip_count),'The new empty box lost its lanes');
    const reloadedBox=await agentCall('arrangement_query',{...boxBinding,box_id:capturedBox.box_id});
    check(JSON.stringify(reloadedBox)===JSON.stringify(capturedBox),'Reboxing changed the frozen inventory');
    const selectionError=async (tool,args,reason)=>{
      const error=await agentCall(tool,args).then(()=>null,error=>error.message);
      check(error?.includes(reason),'Expected '+reason+' from '+tool+' but received '+error);
    };
    await selectionError('ui_selection_set',{...boxBinding,note_ids:[]},'Only the editor');
    await readCurrentBox(4,5.5);
    await selectionError('arrangement_query',{...boxBinding,box_id:capturedBox.box_id,beat_range:[.25,2]},'编排范围在框选之外');
    await selectionError('arrangement_edit',{...boxBinding,...boundCopy,box_id:latestBox.box_id},'编排范围在框选之外');
    await selectionError('arrangement_edit',{...boxBinding,...boundCopy,preview_id:previewBox.preview_id},'PREVIEW_MISMATCH');
    const boxJob=await agentCall('arrangement_audition',{...boxBinding,action:'prepare',engine:'guide',
      box_id:capturedBox.box_id,preview_id:previewBox.preview_id,edit:{kind:'range',edit:boundCopy}});
    const boxAudition={...boxBinding,audition_id:boxJob.job_id};
    let boxSound;
    for(let i=0;i<120;i++){
      boxSound=await agentCall('arrangement_audition',{...boxAudition,action:'status'});
      if(['complete','failed'].includes(boxSound.status))break;
      await new Promise(resolve=>setTimeout(resolve,60));
    }
    check(boxSound?.status==='complete'&&(await invokeNative('verification_audition',
      {audition_id:boxJob.job_id})).distinct_pcm&&project.revision===fitScore.revision,
      'The captured selection did not build a read-only A/B');
    const savedBox=await agentCall('arrangement_audition',{...boxAudition,action:'apply',preview_id:previewBox.preview_id});
    check(savedBox.box_id===capturedBox.box_id&&savedBox.project.revision===fitScore.revision+1&&
      savedBox.project.tracks[1].clips[1].frames===36000&&savedBox.project.tracks[0].notes.length===8,
      'Applying the frozen audition did not use the captured items');
    accept(savedBox.project,true);
    await selectionError('arrangement_query',{project_id:project.id,box_id:capturedBox.box_id},'BOX_STALE');
    await $('undo').onclick();
    check(JSON.stringify(project.tracks)===JSON.stringify(fitScore.tracks),'One undo did not restore the box edit');
    fitScore=project;
    updateArrangeSelection(box);selectClipsInRange(box);
    report.desktop_arrangement_box={published_lanes_and_items:true,empty_lanes:true,rebox_kept_snapshot:true,
      scope_and_stale_rejected:true,external_overwrite_rejected:true,bound_ab_and_apply:true,one_undo:true};
    $('arrangeCopy').click();
    check(rangeDialog.open&&$('arrangeTrackFields').querySelectorAll('input:checked').length===2,
      'The arrangement dialog dropped the selected audio lane');
    $('arrangeTarget').value=4;
    await $('arrangePreview').onclick();
    check(!$('arrangeApply').disabled&&$('arrangePreviewText').textContent.includes('1 个音频片段')&&
      $('arrangePreviewText').textContent.includes('按边界截取 1 段音频')&&
      project.revision===fitScore.revision,'The mixed range could not be previewed as one edit');
    rangeDialog.querySelector('[data-listen]').click();
    await until(()=>rangeAudition.binding()&&audio.auditionId===rangeAudition.binding().audition_id&&
      audio.auditionVariant===1&&!audio.paused,'The range dialog did not play the copied material',25000);
    const rangeBinding=rangeAudition.binding();
    const rangeStatus=await api('arrangement_audition',{...rangeBinding,action:'status'});
    const rangeProof=await invokeNative('verification_audition',{audition_id:rangeBinding.audition_id});
    check(rangeStatus.result.beat_range[0]<4&&rangeStatus.result.beat_range[1]>5.5&&rangeProof.distinct_pcm&&
      project.revision===fitScore.revision,'Range audition did not use the destination and its original context');
    await $('arrangeApply').onclick();
    await until(()=>!rangeDialog.open,'Mixed range copy did not close: '+$('arrangePreviewText').textContent);
    check(trackClips(project.tracks[1]).length===2&&trackClips(project.tracks[1])[0].frames===48000&&
      trackClips(project.tracks[1])[1].start===4&&trackClips(project.tracks[1])[1].offset===6000&&
      trackClips(project.tracks[1])[1].frames===36000&&project.tracks[0].notes.length===8,
      'The mixed range copy did not copy both layers');
    await $('undo').onclick();
    check(JSON.stringify(project.tracks)===JSON.stringify(fitScore.tracks),'Undo did not restore both layers');
    report.desktop_mixed_range={preview_and_commit:true,undo_both_layers:true,sample_window_copy:true,
      direct_destination_ab:true};
    await load(pid);
    if(mixerPanel.hidden)$('showMixer').click();
    await until(()=>!mixerPanel.hidden&&channelViews.size===project.tracks.length,'The mixer did not restore the multi-track fixture');
    await until(()=>!document.querySelector('dialog[open]'),'The previous dialog did not close');
    const ppsfPath=fixtureFolder+'\\桌面_PPSF_迁移验证.ppsf';
    const projectCount=(await api('project_list')).length;
    const importRevision=project.revision;
    const cancelledImport=importNativePaths([ppsfPath]);
    await until(()=>confirmPanel.open,'The desktop PPSF preview did not open');
    const summary=$('studioConfirmText').textContent;
    check(summary.includes('1 条歌声轨 · 2 个音符 · 104 BPM · 6/8')&&
      summary.includes('1 条音频轨')&&summary.includes('2 条旧版 VOCALOID 轨'),
      'PPSF preview did not show source counts and timing');
    confirmPanel.querySelector('[value=no]').click();
    await cancelledImport;
    check(project.id===pid&&project.revision===importRevision&&(await api('project_list')).length===projectCount,
      'Cancelling PPSF preview changed the workspace');
    const confirmedImport=importNativePaths([ppsfPath]);
    await until(()=>confirmPanel.open,'PPSF preview could not be reopened');
    const pickPpsfAudio=async(role,name)=>{
      ppsfReferences.querySelector(`[data-ppsf-role="${role}"]`).click();
      await until(()=>filePanel.open&&fileListing?.directory.endsWith('native_file_fixture'),
        'The audio picker did not show the PPSF folder');
      check($('studioFileEntries').getAttribute('aria-multiselectable')==='false',
        'The reference picker accepted multiple files');
      const entry=[...$('studioFileEntries').querySelectorAll('[data-path]')]
        .find(button=>button.dataset.path.endsWith(name));
      check(entry,`Missing audio reference ${name}`);
      entry.click();$('studioFileApply').click();
      await until(()=>!filePanel.open&&ppsfReferences.querySelector(`[data-ppsf-role="${role}"]`).textContent===name,
        'The selected reference did not return to the PPSF preview');
    };
    await pickPpsfAudio('vocal_path','桌面文件对话框验证.wav');
    ppsfReferences.querySelector('[data-clear-ppsf="vocal_path"]').click();
    check(ppsfReferences.querySelector('[data-ppsf-role="vocal_path"]').textContent==='选择原版人声',
      'Clearing the reference did not update its row');
    await pickPpsfAudio('vocal_path','参考调声验证.wav');
    await pickPpsfAudio('backing_path','格式导入验证_48k.FLAC');
    check((await api('project_list')).length===projectCount,'Selecting reference files created a project');
    $('studioConfirmYes').click();
    await confirmedImport;
    const importedNotes=project.tracks[0].notes;
    check(project.id!==pid&&project.revision===1&&project.bpm===104&&project.meter.join('/')==='6/8'&&
      JSON.stringify(importedNotes.map(n=>[n.start,n.duration,n.pitch,n.lyric]))==='[[1,0.75,64,"あ"],[2,1,67,"い"]]',
      'Confirming PPSF preview did not open an editable copy');
    check(project.tracks.length===3&&project.tracks[0].mute&&!project.tracks[1].mute&&!project.tracks[2].mute&&
      project.tracks[1].name==='原版人声'&&project.tracks[2].name==='伴奏'&&
      project.tracks.slice(1).every(track=>track.audio?.start===0&&track.audio.seconds>0),
      'The selected references did not open as two aligned audio tracks');
    report.desktop_ppsf_preview={shown_before_import:true,cancelled_without_write:true,confirmed_copy_opened:true,
      native_reference_pickers:true,audio_and_score_committed_together:true};
    const originalPpsf=project;
    const previousPitchEngine=$('renderEngine').value;
    $('renderEngine').value='guide';
    await render(false);
    check(rendered?.revision===originalPpsf.revision,'Could not prepare the existing original-stem mix');
    workspaceFocus='editor';
    trackId=project.tracks[0].id;
    chooseGroup(importedNotes.map(note=>note.id));
    $('pitchReference').click();
    await until(()=>$('pitchJoinDialog').open&&!$('pitchReferenceFields').hidden,'Reference tuning did not open');
    $('previewPitchJoin').click();
    await until(()=>!$('applyPitchJoin').disabled,'Reference pitch did not produce a draft');
    check($('pitchJoinStatus').textContent.includes('1 个略过')&&
      $('pitchJoinOriginal').getAttribute('points')!==$('pitchJoinDraft').getAttribute('points')&&
      project.revision===originalPpsf.revision,'The curve draft rewrote the score or included a silent note');
    $('pitchAuditionStart').click();
    await until(()=>pitchAudio?.status==='complete'&&audio.auditionId===pitchAudio.id&&!audio.paused,
      'Reference curves were not audible for the muted score track',25000);
    const referenceAB=(await api('pitch_audition_status',{audition_id:pitchAudio.id})).result;
    const pitchReferenceProof=await invokeNative('verification_audition',{audition_id:pitchAudio.id,hash_audio:true});
    check($('pitchLocalPreview').checked&&referenceAB.source_inputs.join(',')==='local_voice,local_voice'&&!referenceAB.silent.some(Boolean)&&
      pitchReferenceProof.pcm_hashes[0]!==pitchReferenceProof.pcm_hashes[1],
      'Reference comparison reused the original-stem mix or had no audible change');
    const localPair=pitchAudio.id;
    $('pitchLocalPreview').click();
    await until(()=>!audio.auditionId&&!pitchAudio,'The local pair stayed open after switching to full synthesis');
    $('pitchAuditionStart').click();
    await until(()=>pitchAudio?.status==='complete'&&pitchAudio.id!==localPair&&audio.auditionId===pitchAudio.id,
      'The whole-track fallback did not prepare a new pair',25000);
    const fullReferenceAB=(await api('pitch_audition_status',{audition_id:pitchAudio.id})).result;
    check(fullReferenceAB.source_inputs.join(',')==='rendered,rendered'&&
      JSON.stringify(fullReferenceAB.frame_ranges)===JSON.stringify(referenceAB.frame_ranges),
      'Switching modes changed the audition position or used the wrong source');
    $('applyPitchJoin').click();
    await until(()=>!$('pitchJoinDialog').open&&project.revision===originalPpsf.revision+1,
      'Reference curves were not committed in one edit');
    check(project.tracks[0].mute&&project.tracks[0].notes[0].pitch_points.length>10&&
      JSON.stringify(project.tracks[0].notes[1])===JSON.stringify(importedNotes[1])&&
      JSON.stringify(project.tracks.slice(1))===JSON.stringify(originalPpsf.tracks.slice(1)),
      'Reference pitch modified a skipped note, mix state, or original audio');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(originalPpsf.tracks),'Reference curve could not be undone');
    $('renderEngine').value=previousPitchEngine;
    report.desktop_pitch_reference={curves_reviewed:true,isolated_synthesis_ab:true,only_voiced_target_changed:true,single_undo:true};
    report.desktop_local_voice={default_local:true,whole_track_switch:true,same_sample_range:true};
    const releasedSong=(await api('project_list')).find(row=>row.title==='ポケットの天気');
    check(releasedSong,'The published pop project was missing');
    await load(releasedSong.id);
    const sectionScore=project;
    const sectionScope={project_id:sectionScore.id,expected_revision:sectionScore.revision,engine:'voicevox'};
    const beforeSectionJobs=(await api('render_jobs',sectionScope)).jobs;
    check(beforeSectionJobs.some(job=>job.result?.engine==='voicevox'),'No saved mix for the released song');
    const originalSectionEngine=$('renderEngine').value;
    $('renderEngine').value='voicevox';
    workspaceFocus='editor';
    await $('reviewSong').onclick();
    await until(()=>knowledgeDialog.open&&$('sectionContrastOpen'),'Could not enter the score review');
    $('sectionContrastOpen').click();
    await until(()=>$('sectionContrastResult')?.querySelector('table')&&!$('sectionCompare').disabled,
      'The section comparison did not show an initial report');
    for(const [side,part] of [['A','主歌'],['B','副歌']]){
      const select=$('sectionPart'+side);
      select.value=[...select.options].find(option=>option.textContent===part).value;
      select.dispatchEvent(new Event('change'));
    }
    $('sectionCompare').click();
    await until(()=>$('sectionContrastResult').querySelector('.section-compare-summary')?.textContent.includes('副歌')&&
      !$('sectionCompare').disabled,'Verse/chorus comparison did not persist in the panel');
    check($('sectionContrastResult').textContent.includes('清音吉他')&&$('sectionContrastResult').textContent.includes('空隙入音'),
      'Arrangement detail or vocal gaps were missing');
    $('sectionContrastResult').querySelector('[data-section-cue="0"]').click();
    await until(()=>reviewPair?.id&&audio.auditionId===reviewPair.id&&!audio.paused,
      'The real verse and chorus were not auditionable');
    const pairState=await api('range_audition',{action:'status',project_id:sectionScore.id,expected_revision:sectionScore.revision});
    const auditionSection=pairState.pair;
    check(auditionSection.beat_ranges?.[0][0]===8&&auditionSection.beat_ranges?.[1][0]===56,
      'Section audition did not start on the verse and chorus');
    const sectionPCM=await invokeNative('verification_audition',{audition_id:reviewPair.id,hash_audio:true});
    check(sectionPCM.pcm_hashes[0]!==sectionPCM.pcm_hashes[1]&&
      (await api('render_jobs',sectionScope)).jobs.length===beforeSectionJobs.length,
      'The comparison reused the same region or rendered a new mix');
    report.desktop_section_contrast={named_sections:true,precise_openings:true,reused_existing_mix:true};
    const entryCue=$('sectionContrastResult').querySelector('[data-section-kind="lead_entry"]');
    check(entryCue?.textContent.includes('主唱首字')&&$('sectionContrastResult').querySelector('[data-section-kind="lead_release"]'),
      'The section dialog did not expose both independent vocal cues');
    const openingPair=reviewPair.id;
    entryCue.click();
    await until(()=>reviewPair?.id&&reviewPair.id!==openingPair&&audio.auditionId===reviewPair.id&&!audio.paused,
      'The lead-anchored section cue was not played');
    const anchoredPair=(await api('range_audition',{action:'status',project_id:sectionScore.id,
      expected_revision:sectionScore.revision})).pair;
    $('sectionContrastResult').querySelector('[data-section-measure]').click();
    await until(()=>$('sectionContrastResult').querySelector('.section-audio-proof')?.childElementCount>=3,
      'The existing mixed excerpt was not measured');
    const anchoredProof=$('sectionContrastResult').querySelector('.section-audio-proof');
    check(anchoredProof.textContent.includes('LUFS')&&anchoredProof.textContent.includes('主唱相对其余混音')&&
      anchoredPair.saved.job_id===auditionSection.saved.job_id&&
      (await api('render_jobs',sectionScope)).jobs.length===beforeSectionJobs.length,
      'The original-level view changed the render or lacked its vocal balance');
    report.desktop_section_contrast.lead_entry_and_release_cues=true;
    report.desktop_section_contrast.saved_original_levels=true;
    knowledgeDialog.close();
    await until(()=>!knowledgeDialog.open&&!reviewPair&&!audio.auditionId,'Section review kept its temporary audio');
    workspaceFocus='editor';
    await $('reviewSong').onclick();
    await until(()=>knowledgeDialog.open&&$('rhythmInterplayOpen'),'Could not enter rhythm review');
    $('rhythmInterplayOpen').click();
    await until(()=>$('rhythmResult')?.querySelector('table')&&!$('rhythmCompare').disabled,'The rhythm panel did not load');
    $('rhythmFrom').value=56;$('rhythmTo').value=64;
    $('rhythmCompare').click();
    await until(()=>$('rhythmResult')?.textContent.includes('3 对相近落点')&&!$('rhythmCompare').disabled,
      'The report missed the authored chorus drum and bass entrances');
    check($('rhythmResult').textContent.includes('延音'),'The drum report missed sustained bass without a second attack');
    $('rhythmResult').querySelector('[data-rhythm-listen="0"]').click();
    await until(()=>reviewPair?.id&&audio.auditionId===reviewPair.id&&!audio.paused,
      'The actual drums and bass did not play');
    const rhythmPair=(await api('range_audition',{action:'status',project_id:sectionScore.id,expected_revision:sectionScore.revision})).pair;
    const rhythmProof=await invokeNative('verification_audition',{audition_id:reviewPair.id,hash_audio:true});
    check(rhythmPair.saved.outputs[0].length===2&&rhythmPair.saved.outputs[1]===null&&
      rhythmPair.beat_ranges[0].join(',')===rhythmPair.beat_ranges[1].join(',')&&
      rhythmProof.pcm_hashes[0]!==rhythmProof.pcm_hashes[1],'Rhythm-only and full mix previews used the wrong stems');
    check((await api('render_jobs',sectionScope)).jobs.length===beforeSectionJobs.length,'Listening created a redundant render');
    knowledgeDialog.close();
    await until(()=>!knowledgeDialog.open&&!reviewPair&&!audio.auditionId,'The rhythm dialog retained its temporary preview');
    let rhythmFixture=await api('project_create',{title:'鼓贝斯落点验证 · 非成品歌曲',bpm:120});
    rhythmFixture=await api('composition_design',{project_id:rhythmFixture.id,expected_revision:rhythmFixture.revision,
      plan:{purpose:'technical_test',brief:'Bass timing across tempo changes and isolated named notes.'}});
    rhythmFixture=await api('project_edit',{project_id:rhythmFixture.id,expected_revision:rhythmFixture.revision,operations:[
      {op:'set_project',values:{tempo_map:[{beat:4,bpm:90}]}},
      {op:'add_track',track_ref:'rhythmDrumTest',values:{name:'鼓组验证',instrument:{drums:true}}},
      {op:'add_track',track_ref:'rhythmBassTest',values:{name:'贝斯验证',instrument:{program:33}}},
      {op:'add_notes',track_ref:'rhythmDrumTest',notes:[{start:2,duration:.1,pitch:36},{start:3,duration:.1,pitch:36},{start:4,duration:.1,pitch:36}]},
      {op:'add_notes',track_ref:'rhythmBassTest',notes:[{start:1.964,duration:1.2,pitch:43},{start:4.03,duration:.5,pitch:45},{start:5,duration:.25,pitch:48}]}
    ]});
    await load(rhythmFixture.id);
    const rhythmBase=project,rhythmIds=rhythmBase.tracks.slice(1).map(track=>track.id);
    $('renderEngine').value='guide';
    workspaceFocus='editor';
    await $('reviewSong').onclick();
    await until(()=>knowledgeDialog.open&&$('rhythmInterplayOpen'),'Could not reopen the rhythm panel for editing');
    $('rhythmInterplayOpen').click();
    await until(()=>$('rhythmResult')?.querySelector('[data-rhythm-edit]'),'The paired bass move was unavailable');
    $('rhythmResult').querySelector('[data-rhythm-edit]').click();
    $('rhythmOffset').value=12;
    $('rhythmOffset').dispatchEvent(new Event('input'));
    $('rhythmPreview').click();
    await until(()=>$('rhythmApply')&&!$('rhythmApply').disabled,'The proposed bass position was not previewed');
    check(project.revision===rhythmBase.revision&&$('rhythmPlacementState').textContent.includes('→'),
      'Preview wrote an edit or did not show both positions');
    $('rhythmApply').click();
    await until(()=>project.revision===rhythmBase.revision+1&&$('rhythmResult')?.textContent.includes('+12 ms'),
      'The selected bass note did not apply and refresh');
    const rhythmTrackAfter=project.tracks[2],rhythmClockAfter=clockFor(project),shiftedBass=rhythmTrackAfter.notes[0];
    check(Math.abs(rhythmClockAfter.seconds(shiftedBass.start)-rhythmClockAfter.seconds(2)-.012)<1e-8&&
      Math.abs(rhythmClockAfter.seconds(shiftedBass.start+shiftedBass.duration)-rhythmClockAfter.seconds(shiftedBass.start)-.6)<1e-8&&
      JSON.stringify(rhythmTrackAfter.notes.slice(1))===JSON.stringify(rhythmBase.tracks[2].notes.slice(1)),
      'The numeric move affected the bass length or neighbors');
    knowledgeDialog.close();
    await until(()=>!knowledgeDialog.open,'The rhythm editor did not close');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(rhythmBase.tracks),'The rhythm placement did not undo');
    report.desktop_rhythm_interplay={exact_pair_stems:true,existing_mix_reused:true,numeric_preview_and_apply:true,
      only_target_moved:true,one_undo:true};
    $('renderEngine').value=originalSectionEngine;
    await load(tuning.id);
    workspaceFocus='editor';
    const tuningBase=project,tuningTrack=tuningBase.tracks[0];
    const targetNote=tuningTrack.notes[1];
    chooseGroup([targetNote.id]);
    await api('ui_selection_set',{project_id:project.id,expected_revision:project.revision,note_ids:[targetNote.id]});
    const tuningScope={project_id:project.id,expected_revision:project.revision};
    const publicSelection=await agentCall('selection_resolve',{project_id:project.id,from_ui:true});
    check(publicSelection.note_ids.join(',')===targetNote.id,'MCP did not receive the real desktop selection');
    const tuningChange={kind:'vibrato_envelope',settings:{
      depth:63,rate:5.6,start:.24,end:.90,fade_in_ms:80,fade_out_ms:110}};
    const manualPair=await api('pitch_audition_start',{
      selection_id:publicSelection.selection_id,expected_revision:tuningBase.revision,
      transform:tuningChange,engine:'guide',local_voice:true});
    async function waitTuning(status){
      for(let i=0;i<160;i++){
        const job=await status();
        if(job.status==='complete')return job;
        check(['running','committing'].includes(job.status),job.error||'Tuning audition failed');
        await new Promise(resolve=>setTimeout(resolve,80));
      }
      throw new Error('Tuning audition did not finish');
    }
    const manualId=manualPair.job_id;
    await waitTuning(()=>api('pitch_audition_status',{audition_id:manualId}));
    let publicId;
    try{
      await audio.auditionSide(manualId,'A');
      const focus=await agentCall('agent_focus',{...tuningScope,region:'pitch',track_id:tuningTrack.id,note_ids:[targetNote.id],
        reason:'核对选中长音的颤音淡出'});
      const started=await agentCall('tuning_audition',{...tuningScope,action:'prepare',selection_id:publicSelection.selection_id,
        engine:'voicevox',transform:tuningChange,intent_id:focus.intent_id});
      publicId=started.job_id;
      const pair=(await waitTuning(()=>agentCall('tuning_audition',{...tuningScope,action:'status',audition_id:publicId}))).result;
      const now=await transport();
      check(now.audition_id===manualId&&now.playing&&now.owner==='user','Preparing an Agent pair interrupted human playback');
      const agentPlayback=async variant=>{
        const own=await agentCall('desktop_transport',{action:'status'});
        return agentCall('desktop_transport',{...tuningScope,action:'audition_side',audition_id:publicId,
          ticket:own.ticket,variant,reason:'复听选中音符的颤音淡出'});
      };
      let refused='';
      try{await agentPlayback('A')}catch(error){refused=String(error.message)}
      check(refused.includes('USER_TRANSPORT_ACTIVE'),'MCP tuning displaced user playback');
      await audio.stopAudition(manualId);
      let privateRejected='';
      try{
        const own=await agentCall('desktop_transport',{action:'status'});
        await agentCall('desktop_transport',{...tuningScope,action:'audition_side',audition_id:manualId,
          ticket:own.ticket,variant:'A',reason:'核对试听归属'});
      }catch(error){privateRejected=String(error.message)}
      check(privateRejected.includes('试听已更换'),'Agent played a private desktop tuning pair');
      const first=await agentPlayback('A');
      await new Promise(resolve=>setTimeout(resolve,100));
      const second=await agentPlayback('B');
      check(first.owner==='agent'&&second.owner==='agent'&&first.audition_id===publicId&&second.audition_id===publicId&&
        first.audition_variant===0&&second.audition_variant===1&&second.audio_frames>first.audio_frames,
        'Agent tuning sides did not switch on the same native clock');
      const renderedTuning=await invokeNative('verification_audition',{audition_id:publicId,hash_audio:true});
      const measured=await agentCall('tuning_audition',{...tuningScope,action:'measure',audition_id:publicId});
      check(renderedTuning.pcm_hashes[0]!==renderedTuning.pcm_hashes[1]&&measured.source==='prepared_audition_pcm'&&
        measured.frame_ranges[0].join(',')===pair.frame_ranges[0].join(','),'Measured audio differed from the live audition binding');
      const commit={selection_id:publicSelection.selection_id,expected_revision:tuningBase.revision,audition_id:publicId};
      const frozen=await agentCall('selection_edit',commit);
      const saved=await agentCall('selection_edit',{...commit,dry_run:false,intent_id:focus.intent_id});
      check(saved.audition_released&&saved.project.revision===tuningBase.revision+1&&
        JSON.stringify(saved.project.tracks[0].notes[1])===JSON.stringify(frozen.changes[0].after),
        'Agent commit diverged from the auditioned score');
      check(!(await transport()).audition_id&&
        (await api('pitch_audition_status',{audition_id:manualId})).status==='complete','Commit held Agent audio or removed the desktop pair');
      await load(saved.project.id);
      await history(false);
      check(JSON.stringify(project.tracks)===JSON.stringify(tuningBase.tracks),'Auditioned MCP edit is not one undo');
      report.desktop_tuning_audition={captured_real_selection:true,player_respected_user:true,native_ab_switch:true,
        exact_pcm_measured:true,frozen_commit_released:true,one_undo:true};
    }finally{
      await audio.stopAudition(manualId).catch(()=>{});
      await api('pitch_audition_release',{audition_id:manualId}).catch(()=>{});
      if(publicId)await agentCall('tuning_audition',{...tuningScope,action:'release',audition_id:publicId}).catch(()=>{});
    }
    await load(seamScore.id);
    const seamScope={project_id:project.id,expected_revision:project.revision};
    const seamClips=project.tracks.at(-1).clips.map(clip=>({track_id:seamLane,clip_id:clip.id}));
    const seamEdit={clips:seamClips,action:{kind:'crossfade',curve:'linear'}};
    const previewSeam=await api('audio_clip_group',{...seamScope,...seamEdit});
    const remoteSeam=await agentCall('arrangement_audition',{...seamScope,action:'prepare',engine:'guide',
      preview_id:previewSeam.preview_id,edit:{kind:'clip_group',edit:seamEdit}});
    const remoteSeamContext={...seamScope,audition_id:remoteSeam.job_id};
    try{
      await waitTuning(()=>agentCall('arrangement_audition',{...remoteSeamContext,action:'status'}));
      const owner=await agentCall('desktop_transport',{action:'status'});
      const originalSeam=await agentCall('desktop_transport',{...remoteSeamContext,action:'audition_side',
        variant:'A',ticket:owner.ticket,reason:'复听编排接缝原版'});
      const updatedOwner=await agentCall('desktop_transport',{action:'status'});
      const draftedSeam=await agentCall('desktop_transport',{...remoteSeamContext,action:'audition_side',
        variant:'B',ticket:updatedOwner.ticket,reason:'复听编排接缝草稿'});
      check(originalSeam.owner==='agent'&&draftedSeam.owner==='agent'&&
        draftedSeam.audition_id===remoteSeam.job_id&&originalSeam.audition_variant===0&&draftedSeam.audition_variant===1,
        'External Agent could not switch the arrangement audition');
    }finally{await agentCall('arrangement_audition',{...remoteSeamContext,action:'release'})}
    check(!(await transport()).audition_id,'Releasing the external arrangement preview left it playing');
    report.desktop_arrangement_audition={native_agent_switch:true,owned_release:true};
    let beatFade=await api('project_create',{title:'桌面跟速交叉淡化验证 · 非成品歌曲',bpm:120});
    beatFade=await api('composition_design',{project_id:beatFade.id,expected_revision:beatFade.revision,
      plan:{purpose:'technical_test',brief:'Check exact beat-fade markers and the native A/B controls'}});
    beatFade=await api('audio_import',{project_id:beatFade.id,expected_revision:beatFade.revision,
      filename:'跟速接缝.wav',data_base64:btoa(payload),start:0});
    const warpedLane=beatFade.tracks.at(-1).id;
    const divide={project_id:beatFade.id,expected_revision:beatFade.revision,track_id:warpedLane,clip_id:'legacy',
      action:{kind:'split',at:1}};
    const division=await api('audio_clip_edit',divide);
    beatFade=(await api('audio_clip_edit',{...divide,preview_id:division.preview_id})).project;
    const [leftWarp,rightWarp]=beatFade.tracks.at(-1).clips;
    for(const [clip,beatLength,beat] of [[leftWarp,1.45,.55],[rightWarp,1.6,.62]]){
      const warp={project_id:beatFade.id,expected_revision:beatFade.revision,
        clips:[{track_id:warpedLane,clip_id:clip.id}],action:{kind:'warp',beats:beatLength,
          points:[{frame:12000,beat}],allow_overlap:true}};
      const plan=await api('audio_clip_group',warp);
      beatFade=(await api('audio_clip_group',{...warp,preview_id:plan.preview_id})).project;
    }
    await load(beatFade.id);
    const warpedTrack=project.tracks.at(-1);
    setClipGroup(warpedTrack.clips.map(clip=>({track:warpedTrack,clip})));
    workspaceFocus='arrangement';
    $('groupCrossfade').click();
    await until(()=>crossfadeDialog.open&&!$('applyCrossfade').disabled,'Warped overlap did not preview');
    check($('crossfadeLength').disabled&&$('crossfadeLength').valueAsNumber===225,
      'The dialog measured the overlap in source frames instead of project time');
    crossfadeDialog.querySelector('[data-listen]').click();
    await until(()=>crossfadeAudition.binding()&&audio.auditionId===crossfadeAudition.binding().audition_id&&
      audio.auditionVariant===1&&!audio.paused,'Warped crossfade did not play the native draft',25000);
    const warpedBinding=crossfadeAudition.binding();
    const warpedStatus=await api('arrangement_audition',{...warpedBinding,action:'status'});
    const warpedProof=await invokeNative('verification_audition',{audition_id:warpedBinding.audition_id});
    check(warpedStatus.result.transition.time_basis==='beats'&&warpedProof.distinct_pcm&&project.revision===beatFade.revision,
      'The beat-fade A/B pair was not prepared from the exact project overlap');
    $('applyCrossfade').click();
    await until(()=>!crossfadeDialog.open&&project.revision===beatFade.revision+1,'Warped crossfade did not commit');
    const beforeBpm=project;
    const bpmDraft=await api('project_edit',{project_id:beforeBpm.id,expected_revision:beforeBpm.revision,operations:[
      {op:'set_project',values:{tempo_map:[{beat:.65,bpm:96,curve:'linear'},{beat:1.15,bpm:151,curve:'step'}]}}
    ]});
    accept(bpmDraft);
    const afterBpm=(await api('audio_clips',{project_id:project.id})).tracks[0].clips;
    check(afterBpm[0].fade_edges[1]===afterBpm[1].start_frame&&afterBpm[1].fade_edges[0]===afterBpm[0].end_frame&&
      afterBpm[1].start_frame!==warpedStatus.result.transition.frame_range[0],'The tempo change moved one fade away from the other');
    selectAudioClip(project.tracks.at(-1),project.tracks.at(-1).clips[0]);
    $('clipSettings').click();
    await until(()=>clipDialog.open,'Beat fade clip properties did not open');
    check($('clipFadeOutUnit').textContent.includes('拍')&&Math.abs($('clipFadeOut').valueAsNumber-.45)<1e-9&&
      readClipAction().fade_out_beats===project.tracks.at(-1).clips[0].fade_out_beats,
      'Clip properties did not preserve the beat unit');
    clipDialog.close();
    await until(()=>!clipDialog.open,'Beat fade clip properties did not close');
    await history(false);
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(beatFade.tracks),'Undo did not restore both clip maps');
    report.desktop_beat_crossfade={rendered_overlap_ms:true,played_exact_preview:true,retimed_edges:true,
      beat_inspector:true,atomic_undo:true};
    let pitched=await api('project_create',{title:'桌面多轨移调验证 · 非成品歌曲',bpm:120});
    pitched=await api('composition_design',{project_id:pitched.id,expected_revision:pitched.revision,
      plan:{purpose:'technical_test',brief:'Test arrangement transpose scope, range display and native audition'}});
    pitched=await api('project_edit',{project_id:pitched.id,expected_revision:pitched.revision,operations:[
      {op:'add_track',track_ref:'keys',values:{name:'键盘',instrument:{program:0}}},
      {op:'add_track',track_ref:'drum',values:{name:'鼓',instrument:{drums:true}}},
      {op:'add_notes',track_id:pitched.tracks[0].id,notes:[
        {id:'td_0',start:.5,duration:.5,pitch:60,lyric:'あ'},
        {id:'td_1',start:1,duration:1,pitch:64,lyric:'お'},
        {id:'td_out',start:2,duration:.5,pitch:62,lyric:'う'}]},
      {op:'add_notes',track_ref:'keys',notes:[
        {id:'td_keys_low',start:.75,duration:1,pitch:48,lyric:''},
        {id:'td_keys_high',start:.75,duration:1,pitch:55,lyric:''}]},
      {op:'add_notes',track_ref:'drum',notes:[
        {id:'td_drum',start:.25,duration:.5,pitch:38,lyric:''}]}
    ]});
    pitched=await api('audio_import',{project_id:pitched.id,expected_revision:pitched.revision,
      filename:'移调原声.wav',data_base64:btoa(payload),start:.25});
    await load(pitched.id);
    const transposeBox={start:.5,end:2,ids:pitched.tracks.map(track=>track.id)};
    workspaceFocus='arrangement';
    updateArrangeSelection(transposeBox);
    selectClipsInRange(transposeBox);
    check(!$('arrangeTranspose').disabled&&$('arrangeTranspose').getClientRects().length&&
      currentStudioSelection().clip_refs.length===1,'The mixed selection did not keep the arrangement toolbar');
    await EditorCommands.execute('octave_up');
    await until(()=>rangeDialog.open,'The arrangement octave shortcut did not open transpose');
    check($('arrangeShift').valueAsNumber===12&&$('arrangeTargetLabel').hidden,
      'Transpose shortcut did not preserve location or set an octave');
    await $('arrangePreview').onclick();
    const ranges=$('arrangePitchRanges');
    check(!ranges.hidden&&ranges.children.length===2&&ranges.textContent.includes('C4–E4 → C5–E5')&&
      $('arrangePreviewText').textContent.includes('2 轨原音'),'Transpose preview did not show each affected range');
    rangeDialog.querySelector('[data-listen]').click();
    await until(()=>rangeAudition.binding()&&audio.auditionId===rangeAudition.binding().audition_id&&
      audio.auditionVariant===1&&!audio.paused,'Multi-track transpose did not play its exact draft',25000);
    const transposeProof=await invokeNative('verification_audition',{audition_id:rangeAudition.binding().audition_id});
    check(transposeProof.distinct_pcm&&project.revision===pitched.revision,'Transpose audition changed the saved project');
    await $('arrangeApply').onclick();
    await until(()=>!rangeDialog.open&&project.revision===pitched.revision+1,'Transpose did not commit');
    check(project.tracks[0].notes.map(note=>note.pitch).join(',')==='72,76,62'&&
      project.tracks[1].notes.map(note=>note.pitch).join(',')==='60,67'&&project.tracks[2].notes[0].pitch===38&&
      JSON.stringify(project.tracks[3])===JSON.stringify(pitched.tracks[3]),
      'Transpose applied to the wrong parts or altered drum or audio tracks');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(pitched.tracks),'Multi-track transpose was not one undo step');
    report.desktop_range_transpose={shortcut_and_tonal_ranges:true,exact_native_ab:true,scoped_apply_and_undo:true};
    const timingBox={...transposeBox,start:0};
    workspaceFocus='arrangement';
    updateArrangeSelection(timingBox);
    key(arrangeViewport,'q');
    await until(()=>rangeDialog.open,'Arrangement Q did not open multi-track quantization');
    check(!$('arrangeTimingFields').hidden&&$('arrangeShiftLabel').hidden&&
      $('arrangeQuantize').getBoundingClientRect().width>0,'Quantize controls were not visible');
    $('arrangeGrid').value='0.25';
    $('arrangeStrength').value='75';
    $('arrangeSwing').value='65';
    await $('arrangePreview').onclick();
    check(!ranges.hidden&&ranges.children.length===2&&ranges.textContent.includes('28.1 ms')&&
      $('arrangePreviewText').textContent.includes('1 段音频原位'),'Quantize preview missed the timings or fixed audio');
    const beforeTiming=project.revision;
    rangeDialog.querySelector('[data-listen]').click();
    await until(()=>rangeAudition.binding()&&audio.auditionId===rangeAudition.binding().audition_id&&
      audio.auditionVariant===1&&!audio.paused,'Multi-track quantize did not play its exact draft',25000);
    const timingProof=await invokeNative('verification_audition',{audition_id:rangeAudition.binding().audition_id});
    check(timingProof.distinct_pcm&&project.revision===beforeTiming,'Quantize audition saved before approval');
    await $('arrangeApply').onclick();
    await until(()=>!rangeDialog.open&&project.revision===beforeTiming+1,'Quantize did not commit once');
    check(project.tracks[1].notes.every(note=>Math.abs(note.start-.80625)<1e-8)&&
      Math.abs(project.tracks[2].notes[0].start-.30625)<1e-8&&
      JSON.stringify(project.tracks[0])===JSON.stringify(pitched.tracks[0])&&
      JSON.stringify(project.tracks[3])===JSON.stringify(pitched.tracks[3]),'Quantize altered the wrong notes or audio');
    await history(false);
    check(JSON.stringify(project.tracks)===JSON.stringify(pitched.tracks),'Multi-track quantize was not one undo step');
    report.desktop_range_quantize={shortcut_and_track_timing:true,exact_native_ab:true,scoped_apply_and_undo:true};
    const installed=await api('studio_status');
    if(installed.openutau?.configured){
      await until(()=>$('renderEngine').querySelector('[value=openutau]'),'OpenUtau was absent from the local-engine menu');
      let voiceTest=await api('project_create',{title:'桌面 OpenUtau 验证 · 非成品歌曲',bpm:120});
      voiceTest=await api('composition_design',{project_id:voiceTest.id,expected_revision:voiceTest.revision,
        plan:{purpose:'technical_test',brief:'Verify the installed OpenUtau engine in the desktop transport'}});
      voiceTest=await api('project_edit',{project_id:voiceTest.id,expected_revision:voiceTest.revision,operations:[
        {op:'add_notes',track_id:voiceTest.tracks[0].id,notes:[
          {start:0,duration:1.5,pitch:64,lyric:'か'},
          {start:1.5,duration:1.5,pitch:67,vowel_continuation:true}]}]});
      await load(voiceTest.id);
      const priorEngine=$('renderEngine').value;
      try{
        $('renderEngine').value='openutau';
        $('play').click();
        await until(()=>audio.liveEngine==='openutau'&&!audio.paused,
          'The desktop did not play the installed Classic voicebank',25000);
        await render(false);
        check(rendered.revision===voiceTest.revision&&rendered.audio_verified_non_silent&&
          rendered.credits.some(credit=>credit.includes(installed.openutau.name)),
          'Desktop OpenUtau source, credits or score revision were inconsistent');
        $('stop').click();
        await until(()=>audio.paused,'Native OpenUtau playback did not stop');
        report.desktop_openutau={selectable:true,played_native:true,credited_real_bank:true};
        if(installed.openutau.profiles.some(profile=>profile.id==='teto_whisper')){
          voiceTest=await api('project_edit',{project_id:voiceTest.id,expected_revision:voiceTest.revision,operations:[
            {op:'add_track',track_ref:'harmony',values:{name:'测试和声',gain_db:-8}},
            {op:'add_notes',track_ref:'harmony',notes:[{start:.5,duration:1.5,pitch:60,lyric:'あ'}]}]});
          await load(voiceTest.id);
          const main=voiceTest.tracks[0].id, harmony=voiceTest.tracks[1].id;
          trackId=main;
          refresh();
          $('singer').value='openutau:default';
          $('singer').dispatchEvent(new Event('change',{bubbles:true}));
          await until(()=>project.tracks[0].openutau_voice==='default','The inspector did not save the main voice');
          check($('renderEngine').value==='project'&&!project.tracks[1].openutau_voice,
            'The voice picker changed the wrong track or kept the forced render mode');
          $('play').click();
          await until(()=>audio.liveEngine==='project'&&!audio.paused,
            'The desktop did not play independent track voices',25000);
          await render(false);
          check(rendered.voice_sources[main].id==='default'&&rendered.voice_sources[harmony].engine==='voicevox',
            'Desktop mixed render used the wrong providers');
          $('stop').click();
          await until(()=>audio.paused&&!rendering,'Desktop mixed playback did not settle');
          report.desktop_track_voices={inspector_assignment:true,played_mixed_sources:true};
        }
      }finally{$('renderEngine').value=priorEngine}
    }
    const receiving=await api('project_create',{title:'桌面分轨导入验证 · 非成品歌曲',bpm:120});
    await load(receiving.id);
    $('importBtn').click();
    await inAppFolder();
    const archiveChoice=[...$('studioFileEntries').querySelectorAll('[data-path]')]
      .find(button=>button.dataset.path===report.desktop_stem_delivery);
    check(archiveChoice,'Studio stem ZIP was not visible in its native picker');
    archiveChoice.click();$('studioFileApply').click();
    await until(()=>!filePanel.open&&project?.id===receiving.id&&project.revision>receiving.revision,
      'The stem ZIP was not imported');
    const importedStems=project.tracks.slice(1);
    check(project.revision===receiving.revision+1&&importedStems.length>1&&
      importedStems.every(track=>track.clips?.length===1&&track.clips[0].start===0&&
        track.clips[0].frames===importedStems[0].clips[0].frames),'The native stem import did not keep the tracks aligned');
    await history(false);
    check(project.tracks.length===1,'The native stem import required more than one undo');
    report.desktop_stem_import={aligned:true,tracks:importedStems.length,one_undo:true};
    await load(pid);
    report.project_id=pid;
    report.completed=true;
  }catch(error){report.completed=false;report.error=String(error.stack||error)}
  await invokeNative('verification_result',report);
})();

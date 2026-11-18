void (async()=>{
  const report={surfaces:{},failures:[]};
  const rgba=value=>{
    if(value==='transparent')return [0,0,0,0];
    if(/^#[0-9a-f]{6}$/i.test(value))return [...value.slice(1).match(/../g).map(channel=>parseInt(channel,16)),1];
    const channels=value.match(/[\d.]+/g)?.map(Number);
    return channels?.length>=3?[...channels.slice(0,3),channels[3]??1]:null;
  };
  const luminance=([r,g,b])=>[r,g,b].reduce((sum,channel,index)=>{
    const s=channel/255;
    return sum+[.2126,.7152,.0722][index]*(s<=.04045?s/12.92:((s+.055)/1.055)**2.4);
  },0);
  const blend=(fg,bg)=>fg.slice(0,3).map((c,i)=>c*fg[3]+bg[i]*(1-fg[3]));
  const ratio=(a,b)=>{
    const values=[luminance(a),luminance(b)].sort((x,y)=>x-y);
    return (values[1]+.05)/(values[0]+.05);
  };
  const eachFrame=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  function audit(root,name){
    const measurements={text:0,icons:0,control_boundaries:0,focus:0,minimum:{},glass:false};
    report.surfaces[name]=measurements;
    measurements.glass=getComputedStyle(root).backgroundImage.includes('gradient');
    const backgroundCache=new Map();
    function background(el){
      if(!el)return [[16,16,16],[16,16,16]];
      if(backgroundCache.has(el))return backgroundCache.get(el);
      const style=getComputedStyle(el),fill=rgba(style.backgroundColor);
      // A floating surface can sit over the brightest possible content.
      const back=style.backdropFilter!=='none'?[[0,0,0],[255,255,255]]:background(el.parentElement);
      const base=fill?back.map(bg=>blend(fill,bg)):back;
      const stops=style.backgroundImage.startsWith('linear-gradient')
        ?[...style.backgroundImage.matchAll(/rgba?\([^)]*\)/g)].map(m=>rgba(m[0])).filter(Boolean):[];
      const combinations=[...base,...base.flatMap(bg=>stops.map(stop=>blend(stop,bg)))];
      combinations.sort((a,b)=>luminance(a)-luminance(b));
      const extremes=[combinations[0],combinations.at(-1)];
      backgroundCache.set(el,extremes);
      return extremes;
    }
    function threshold(el,style,key,foreground,backgrounds,limit,content){
      if(!foreground)return;
      const color=[...foreground];
      let cursor=el;
      while(cursor){color[3]*=Number(getComputedStyle(cursor).opacity);cursor=cursor.parentElement}
      const value=Math.min(...backgrounds.map(bg=>ratio(blend(color,bg),bg)));
      measurements[key]++;
      measurements.minimum[key]=Number(Math.min(measurements.minimum[key]??21,value).toFixed(2));
      if(value+.005<limit)report.failures.push({
        surface:name,type:key,ratio:Number(value.toFixed(2)),threshold:limit,
        element:el.id||el.getAttribute('aria-label')||el.classList[0]||el.localName,
        sample:content?.slice(0,38),color:style.color,backgrounds
      });
    }
    function visible(el){
      return el.getClientRects().length>0
        && !el.closest('[hidden], [inert], [aria-hidden=true], :disabled, [aria-disabled=true], #noteInspector.disabled input, #noteInspector.disabled button')
        && getComputedStyle(el).visibility==='visible'&&Number(getComputedStyle(el).opacity)>0;
    }
    const elements=[root,...root.querySelectorAll('*')];
    for(const el of elements){
      if(!visible(el)||el instanceof SVGElement||el.matches('script,style,option'))continue;
      const style=getComputedStyle(el);
      const backgrounds=background(el);
      const control=el.matches('input,textarea,select');
      let content=[...el.childNodes].filter(node=>node.nodeType===Node.TEXT_NODE).map(node=>node.textContent.trim()).filter(Boolean).join(' ');
      let textStyle=style;
      if(el.matches('select'))content=el.selectedOptions[0]?.textContent||'';
      if(el.matches('textarea,input:not([type=range],[type=checkbox],[type=color],[type=radio],[type=hidden])')){
        content=el.value||el.placeholder;
        if(!el.value&&el.placeholder)textStyle=getComputedStyle(el,'::placeholder');
      }
      if(content){
        const size=parseFloat(style.fontSize),weight=Number(style.fontWeight);
        threshold(el,textStyle,'text',rgba(textStyle.color),backgrounds,size>=24||size>=18.66&&weight>=700?3:4.5,content);
      }
      if(el.matches('button,summary,a,[role=button]')&&!content){
        const svg=el.querySelector('svg');
        if(svg){
          const stroke=getComputedStyle(svg.querySelector('path,polygon,circle,rect')||svg);
          threshold(el,stroke,'icons',rgba(stroke.stroke)||rgba(stroke.fill)||rgba(style.color),backgrounds,3);
        }
      }
      if(control&&el.matches('input:not([type=range],[type=color],[type=checkbox],[type=radio]),textarea,select')){
        const base=background(el.parentElement),border=rgba(style.borderTopColor);
        const visibleBorder=border&&parseFloat(style.borderTopWidth)>0&&style.borderTopStyle!=='none';
        const fill=rgba(style.backgroundColor);
        const indicator=visibleBorder&&border[3]>.2?border:fill;
        // Text on an integrated toolbar control is a visual identifier. Framed fields require a visible edge.
        if(visibleBorder&&border[3]>.2&&style.borderTopStyle==='solid')threshold(el,style,'control_boundaries',indicator,base,3);
      }
    }
    const targets=elements.filter(el=>el.matches('button,input:not([type=hidden]),select,textarea,summary,a,[role=separator]')&&visible(el));
    // Probe keyboard-style focus on the common control types without re-running the entire DOM layout per item.
    const previous=document.activeElement;
    const firstOfType=new Set();
    for(const el of targets){
      const type=el.localName+':'+(el.getAttribute('type')||'');
      if(firstOfType.has(type))continue;
      firstOfType.add(type);
      el.focus({preventScroll:true,focusVisible:true});
      const style=getComputedStyle(el);
      if(style.outlineStyle!=='none'&&parseFloat(style.outlineWidth)>0){
        threshold(el,style,'focus',rgba(style.outlineColor),background(el.parentElement),3);
      }else if(el.matches(':focus-visible')){
        report.failures.push({surface:name,type:'focus',element:el.id||type,reason:'No visible focus outline'});
      }
    }
    previous?.focus?.({preventScroll:true});
  }
  try{
    report.host_version=(await invokeNative('host_info')).version;
    const oldMotion=motionPreference;
    motionPreference='reduced';
    setMotion();
    let fixture=await api('project_create',{title:'深色无障碍配色验证 · 非成品歌曲',bpm:120});
    fixture=await api('composition_design',{project_id:fixture.id,expected_revision:fixture.revision,
      plan:{purpose:'technical_test',brief:'Measure text, controls and focus on native dark-glass surfaces.'}});
    fixture=await api('project_edit',{project_id:fixture.id,expected_revision:fixture.revision,operations:[
      {op:'add_notes',track_id:fixture.tracks[0].id,notes:[{start:1,duration:1,pitch:60,lyric:'あ'},{start:2,duration:1,pitch:64,lyric:'い'}]}
    ]});
    await load(fixture.id);
    chooseGroup([fixture.tracks[0].notes[0].id]);
    await eachFrame();
    await new Promise(resolve=>setTimeout(resolve,100));
    audit(studioMain,'main-workspace');
    const contexts=new Set([roll,expr,arrangeCanvas]);
    const source=CanvasRenderingContext2D.prototype;
    const paintText=source.fillText;
    const canvas={labels:0,track_colors:0,minimum_text:21,minimum_track_edge:21};
    report.canvas=canvas;
    source.fillText=function(label,x,y,...args){
      if(contexts.has(this.canvas)){
        const fg=rgba(this.fillStyle),matrix=this.getTransform();
        const sample=matrix.transformPoint({x:x+1,y:y-5});
        if(fg&&sample.x>=0&&sample.y>=0&&sample.x<this.canvas.width&&sample.y<this.canvas.height){
          const pixel=this.getImageData(Math.floor(sample.x),Math.floor(sample.y),1,1).data;
          if(pixel[3]===255){
            fg[3]*=this.globalAlpha;
            const bg=[...pixel].slice(0,3),value=ratio(blend(fg,bg),bg);
            canvas.labels++;
            canvas.minimum_text=Math.min(canvas.minimum_text,value);
            if(value<4.5)report.failures.push({surface:this.canvas.id,type:'canvas-text',sample:String(label),ratio:Number(value.toFixed(2))});
          }
        }
      }
      return paintText.call(this,label,x,y,...args);
    };
    try{draw();drawArrangement()}finally{source.fillText=paintText}
    for(const color of trackColors){
      canvas.track_colors++;
      for(const selected of [false,true]){
        const palette=scoreNotePalette(color,selected);
        const text=ratio(rgba(palette.ink),rgba(palette.fill)),edge=ratio(rgba(palette.fill),[16,16,18]);
        canvas.minimum_text=Math.min(canvas.minimum_text,text);
        canvas.minimum_track_edge=Math.min(canvas.minimum_track_edge,edge);
        if(text<4.5||edge<3)report.failures.push({surface:'score-track-palette',sample:color,type:'canvas-track',ratio:Number(Math.min(text,edge).toFixed(2))});
      }
    }
    canvas.minimum_text=Number(canvas.minimum_text.toFixed(2));
    canvas.minimum_track_edge=Number(canvas.minimum_track_edge.toFixed(2));
    if(typeof mixerPanel!=='undefined'&&mixerPanel.hidden){
      mixerButton.click();
      await eachFrame();
      await new Promise(resolve=>setTimeout(resolve,110));
      audit(mixerPanel,'mixer');
      mixerButton.click();
      await eachFrame();
    }
    const dialogs=[...document.querySelectorAll('dialog')];
    for(const [index,dialog] of dialogs.entries()){
      if(dialog===commandDialog)openCommands(false);
      else if(dialog===knowledgeDialog)await showKnowledge();
      else if(dialog===timelineDialog)$('showTimeline').click();
      else if(dialog===fxDialog)$('showEffects').click();
      else if(dialog===waveExportDialog)showWaveExport({stems:['01_主唱.wav','02_伴奏.wav']},[1,4]);
      else if(dialog===sectionDialog){
        showSections();
        addSectionRow({name:'副歌',start:0,end:4});
      }
      else if(dialog===outputDialog)await openOutputSettings();
      else HTMLDialogElement.prototype.showModal.call(dialog);
      await eachFrame();
      if(!dialog.open)HTMLDialogElement.prototype.showModal.call(dialog);
      for(const animation of dialog.getAnimations({subtree:true})){
        if(Number.isFinite(animation.effect?.getComputedTiming().endTime))animation.finish();
      }
      audit(dialog,dialog.id||[...dialog.classList].join('-')||'dialog-'+index);
      if(dialog.open)HTMLDialogElement.prototype.close.call(dialog);
      if(dialog===clipDialog){
        $('clipKind').value='trim';
        refreshClipFields();
        HTMLDialogElement.prototype.showModal.call(dialog);
        await eachFrame();
        for(const animation of dialog.getAnimations({subtree:true})){
          if(Number.isFinite(animation.effect?.getComputedTiming().endTime))animation.finish();
        }
        audit(dialog,'clipTrimControls');
        HTMLDialogElement.prototype.close.call(dialog);
        $('clipKind').value='set';
        refreshClipFields();
      }
    }
    motionPreference=oldMotion;
    setMotion();
    report.completed=true;
    report.passed=report.failures.length===0;
  }catch(error){report.completed=false;report.error=String(error.stack||error)}
  await invokeNative('verification_theme_result',report);
})();

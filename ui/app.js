/* 行为生态缸 —— 状态、交互与渲染循环 */
(() => {
  const W = 224;
  const H = 300;
  const hasTauri = typeof window.__TAURI__ !== 'undefined';
  const invoke = (cmd, args) =>
    hasTauri ? window.__TAURI__.core.invoke(cmd, args) : Promise.resolve(null);

  const blank = () => ({
    stage: 0,
    stage_name: '孢子',
    growth: 0,
    paused: false,
    demo: false,
    active: true,
    caption: '它还在适应这里。',
    active_minutes: 0,
    focus_minutes: 0,
    last_break_ms: 0,
    idle_sec: 0,
    fg_app: null,
    uptime_sec: 0,
    diag_open: false,
    buckets60: [],
    day_active_min: 0,
    day_breaks: 0,
    autostart_state: 0,
    ui_scale: 1,
    opacity: 1,
    collapsed: false,
    data_dir: '',
    log_path: '',
    traits: { vitality: 0.55, glow: 0.12, hue: 0.15, turbidity: 0.35, motility: 0.3, rhythm: 0.3, spots: 0 },
    sig: { focus: 0, rhythm: 0.35, calm: 0.6, load: 0, variety: 0, active_ratio: 0, streak_min: 0, rest_min: 0 },
  });

  const cur = blank();
  const tgt = blank();

  const el = {
    card: document.getElementById('card'),
    stageName: document.getElementById('stage-name'),
    stageMeta: document.getElementById('stage-meta'),
    caption: document.getElementById('caption'),
    today: document.getElementById('today'),
    paused: document.getElementById('paused-badge'),
    menu: document.getElementById('menu'),
    about: document.getElementById('about'),
    diag: document.getElementById('diag'),
    toast: document.getElementById('toast'),
    d: {
      state: document.getElementById('d-state'),
      uptime: document.getElementById('d-uptime'),
      idle: document.getElementById('d-idle'),
      input: document.getElementById('d-input'),
      fg: document.getElementById('d-fg'),
      auto: document.getElementById('d-auto'),
      tray: document.getElementById('d-tray'),
      bars: document.getElementById('d-bars'),
      day: document.getElementById('d-day'),
      breaks: document.getElementById('d-breaks'),
      data: document.getElementById('d-data'),
    },
    codex: document.getElementById('codex'),
    codexCount: document.getElementById('codex-count'),
    codexSlots: document.getElementById('codex-slots'),
    codexGens: document.getElementById('codex-gens'),
    codexClose: document.getElementById('codex-close'),
    bars: {
      focus: document.getElementById('b-focus'),
      rhythm: document.getElementById('b-rhythm'),
      calm: document.getElementById('b-calm'),
      load: document.getElementById('b-load'),
      variety: document.getElementById('b-variety'),
    },
  };

  let hover = false;
  let menuOpen = false;
  let aboutOpen = false;
  let diagOpen = false;
  let uiScale = 1;
  let uiOpacity = 1;
  let uiCollapsed = false;
  let diagBars = [];
  let lastBreak = -1;
  let lastStage = 0;
  let lastCard = 0;
  let toastTimer = 0;
  let lastTs = 0;
  let lastDraw = 0;
  let dtDraw = 1 / 30;
  let uiGenome = null;
  let uiArch = 'medusa';
  let uiSpeciesId = null;
  let lastSpecies = null;
  let codexOpen = false;
  let previewSpec = null;
  let previewUntil = 0;

  Scene.init(document.getElementById('stage'));

  /* ---------- 状态接收 ---------- */

  function applySnapshot(s) {
    tgt.stage = s.stage;
    tgt.stage_name = s.stage_name;
    tgt.growth = s.growth;
    tgt.paused = s.paused;
    tgt.demo = s.demo;
    tgt.active = s.active !== false;
    tgt.caption = s.caption;
    tgt.active_minutes = s.active_minutes;
    tgt.last_break_ms = s.last_break_ms;
    Object.assign(tgt.traits, s.traits);
    Object.assign(tgt.sig, s.sig);

    if (lastBreak < 0) lastBreak = s.last_break_ms;
    if (s.last_break_ms !== lastBreak) {
      lastBreak = s.last_break_ms;
      Scene.burst(9);
    }
    if (s.stage > lastStage) {
      lastStage = s.stage;
      Scene.burst(20);
      toast('它长大了：' + s.stage_name);
      invoke('set_menu_open', { open: menuOpen || aboutOpen });
    } else if (s.stage < lastStage) {
      lastStage = s.stage;
    }

    el.paused.classList.toggle('show', s.paused);
    el.menu.querySelector('[data-act="pause"]').textContent = s.paused ? '继续观察' : '暂停观察';
    el.menu.querySelector('[data-act="autostart"]').textContent =
      '开机自启：' + (['关闭', '已开启', '被拦截'][s.autostart_state || 0] || '关闭');
    const sc = s.ui_scale || 1;
    const scIdx = sc < 0.9 ? 0 : sc < 1.12 ? 1 : 2;
    el.menu.querySelector('[data-act="scale"]').textContent =
      '大小：' + ['迷你', '紧凑', '标准'][scIdx];
    el.menu.querySelector('[data-act="opacity"]').textContent =
      '不透明度：' + Math.round((s.opacity == null ? 1 : s.opacity) * 100) + '%';
    if (s.collapsed) closeMenu();

    tgt.idle_sec = s.idle_sec || 0;
    tgt.fg_app = s.fg_app || null;
    tgt.uptime_sec = s.uptime_sec || 0;
    tgt.day_active_min = s.day_active_min || 0;
    tgt.day_breaks = s.day_breaks || 0;
    tgt.autostart_state = s.autostart_state || 0;
    tgt.data_dir = s.data_dir || '';
    tgt.log_path = s.log_path || '';
    tgt.buckets60 = s.buckets60 || [];

    tgt.ui_scale = s.ui_scale || 1;
    tgt.opacity = s.opacity == null ? 1 : s.opacity;
    tgt.collapsed = !!s.collapsed;
    applyUi(s);

    applyDiag(!!s.diag_open, true);
    updateDiag(s);

    // 世代:基因/图纸随快照下发,显形瞬间给提示。
    uiGenome = s.genome || uiGenome;
    uiArch = s.archetype || 'medusa';
    uiSpeciesId = s.species ? s.species.id : null;
    const sp = s.species || null;
    if (sp && !lastSpecies) {
      lastSpecies = sp;
      toast('显形:' + sp.name);
      Scene.burst(26);
    }
    if (!sp) lastSpecies = null;
    tgt.can_collect = !!s.can_collect;
    const collectItem = el.menu.querySelector('[data-act="collect"]');
    if (collectItem) {
      collectItem.textContent = s.can_collect ? '收集它' : '收集(未定形)';
      collectItem.classList.toggle('disabled', !s.can_collect);
    }

    hover = !!s.hover;
    syncCard();
  }

  if (hasTauri) {
    // 轮询快照：不依赖 Tauri 的事件权限，行为更可控。
    let inflight = false;
    const pull = async () => {
      if (inflight) return;
      inflight = true;
      try {
        const s = await invoke('get_snapshot');
        if (s) applySnapshot(s);
      } catch (e) {
        /* 启动瞬间可能尚未就绪 */
      }
      inflight = false;
    };
    pull();
    setInterval(pull, 400);
  } else {
    previewMode();
  }

  /* ---------- 图鉴 ---------- */

  function openCodex() {
    if (codexOpen) return;
    codexOpen = true;
    closeMenu();
    el.codex.classList.add('show');
    invoke('set_codex', { open: true });
    invoke('get_codex')
      .then(renderCodex)
      .catch(() => {});
  }

  function closeCodex() {
    if (!codexOpen) return;
    codexOpen = false;
    el.codex.classList.remove('show');
    invoke('set_codex', { open: false });
  }

  function renderCodex(d) {
    if (!d) return;
    const got = d.slots.filter((s) => s.collected).length;
    el.codexCount.textContent = `已收集 ${got} / ${d.slots.length} 种`;
    el.codexSlots.innerHTML = '';
    for (const s of d.slots) {
      const div = document.createElement('div');
      div.className = 'slot' + (s.collected ? ' got' : '');
      const nm = document.createElement('b');
      nm.textContent = s.collected ? s.name : '???';
      const hint = document.createElement('i');
      hint.textContent = s.hint;
      div.appendChild(nm);
      div.appendChild(hint);
      if (s.collected && s.count > 1) {
        const em = document.createElement('em');
        em.textContent = '×' + s.count;
        div.appendChild(em);
      }
      el.codexSlots.appendChild(div);
    }
    el.codexGens.innerHTML = '';
    const gens = (d.generations || []).slice().reverse();
    if (!gens.length) {
      const empty = document.createElement('div');
      empty.className = 'gen-empty';
      empty.textContent = '还没有收集过。成体之后右键"收集它"。';
      el.codexGens.appendChild(empty);
      return;
    }
    for (const g of gens) {
      const row = document.createElement('div');
      row.className = 'gen-row';
      const b = document.createElement('b');
      b.textContent = `第${g.gen}代`;
      const sp = document.createElement('span');
      sp.className = 'sp';
      sp.textContent = g.species_name;
      const rng = document.createElement('span');
      rng.className = 'rng';
      rng.textContent = `${g.from} ~ ${g.to}`;
      const hrs = document.createElement('span');
      hrs.className = 'hrs';
      hrs.textContent =
        g.active_min >= 60 ? (g.active_min / 60).toFixed(1) + ' 小时' : Math.round(g.active_min) + ' 分钟';
      row.appendChild(b);
      row.appendChild(sp);
      row.appendChild(rng);
      row.appendChild(hrs);
      if (g.marks && g.marks.length) {
        const mk = document.createElement('span');
        mk.className = 'mk';
        mk.textContent = g.marks.join(' · ');
        row.appendChild(mk);
      }
      row.addEventListener('mousedown', (e) => {
        e.stopPropagation();
        visitGen(g);
      });
      el.codexGens.appendChild(row);
    }
  }

  // 探视:用冻存的基因与外观,让已收集的生物在缸里再游 9 秒。
  function visitGen(g) {
    closeCodex();
    previewSpec = {
      stage: 3,
      growth: 0.6,
      traits: {
        vitality: g.visual ? g.visual.vitality : 0.7,
        glow: g.visual ? g.visual.glow : 0.5,
        hue: g.visual ? g.visual.hue : 0.2,
        turbidity: g.visual ? g.visual.turbidity : 0.2,
        motility: g.visual ? g.visual.motility : 0.6,
        rhythm: g.visual ? g.visual.rhythm_t : 0.5,
        spots: g.visual ? g.visual.spots : 0,
      },
      sig: {},
      genome: g.genome,
      archetype: g.archetype,
      species_id: g.species_id,
      paused: false,
      active: true,
      demo: true,
    };
    previewUntil = performance.now() + 9000;
    toast(`探视:第${g.gen}代 · ${g.species_name}`);
  }

  el.codexClose.addEventListener('mousedown', (e) => {
    e.stopPropagation();
    closeCodex();
  });

  /* ---------- 悬停 / 菜单 ---------- */

  function syncCard() {
    const show = hover || menuOpen || aboutOpen;
    el.card.classList.toggle('show', show);
  }

  /* ---------- 尺寸 / 透明度 / 收起 ---------- */

  function applyUi(s) {
    const app = document.getElementById('app');
    const dock = document.getElementById('dock');
    const scale = s.ui_scale || 1;
    const alpha = s.opacity == null ? 1 : s.opacity;
    const collapsed = !!s.collapsed;

    if (Math.abs(scale - uiScale) > 0.001) {
      uiScale = scale;
      app.style.transform = `scale(${scale})`;
      Scene.setScale(scale);
    }
    if (Math.abs(alpha - uiOpacity) > 0.001) {
      uiOpacity = alpha;
      app.style.opacity = alpha;
      dock.style.opacity = Math.max(0.5, alpha);
    }
    if (collapsed !== uiCollapsed) {
      uiCollapsed = collapsed;
      document.body.classList.toggle('collapsed', collapsed);
      if (!collapsed) Scene.setScale(uiScale);
    }
    if (document.body.classList.contains('boot')) {
      document.body.classList.remove('boot');
    }
  }

  /* ---------- 自检面板 ---------- */

  function applyDiag(on, fromServer) {
    if (on === diagOpen) return;
    diagOpen = on;
    el.diag.classList.toggle('show', on);
    if (on) {
      aboutOpen = false;
      el.about.classList.remove('show');
      ensureDiagBars();
    }
    syncCard();
    if (!fromServer) invoke('set_diag', { open: on });
  }

  function ensureDiagBars() {
    if (diagBars.length) return;
    for (let i = 0; i < 60; i++) {
      const b = document.createElement('i');
      el.d.bars.appendChild(b);
      diagBars.push(b);
    }
  }

  function fmtDuration(sec) {
    if (sec < 60) return Math.round(sec) + ' 秒';
    if (sec < 3600) return Math.round(sec / 60) + ' 分钟';
    return Math.floor(sec / 3600) + ' 小时 ' + Math.round((sec % 3600) / 60) + ' 分';
  }

  function shortPath(p) {
    if (!p) return '—';
    return p.length > 46 ? '…' + p.slice(-45) : p;
  }

  function updateDiag(s) {
    if (!diagOpen) return;
    el.d.state.textContent = s.paused ? '已暂停' : '观察中';
    el.d.uptime.textContent = fmtDuration(s.uptime_sec || 0);
    el.d.idle.textContent = (s.idle_sec || 0) + 's';
    el.d.input.textContent = Math.round((s.sig.input_ratio || 0) * 100) + '%';
    el.d.fg.textContent = s.fg_app || '—';
    el.d.auto.textContent = ['关闭', '已登记', '被拦截'][s.autostart_state || 0] || '关闭';
    el.d.tray.textContent = s.tray_ok ? '正常' : '被拦截';
    el.d.day.textContent = `今日 ${Math.round(s.day_active_min || 0)} 分钟`;
    el.d.breaks.textContent = `${s.day_breaks || 0} 次休息`;
    el.d.data.textContent = s.data_dir ? '本地 data/' : '—';
    el.d.data.title = s.data_dir || '';
    ensureDiagBars();
    const b = s.buckets60 || [];
    for (let i = 0; i < diagBars.length; i++) {
      const v = b[i] || 0;
      diagBars[i].style.height = 2 + (v / 60) * 26 + 'px';
      diagBars[i].style.opacity = v ? 0.35 + (v / 60) * 0.65 : 0.12;
    }
  }

  function openMenu(x, y) {
    aboutOpen = false;
    el.about.classList.remove('show');
    el.menu.classList.add('show');
    const w = el.menu.offsetWidth || 132;
    const h = el.menu.offsetHeight || 148;
    el.menu.style.left = Math.max(6, Math.min(x, W - w - 6)) + 'px';
    el.menu.style.top = Math.max(6, Math.min(y, H - h - 6)) + 'px';
    menuOpen = true;
    syncCard();
    invoke('set_menu_open', { open: true });
  }

  function closeMenu() {
    if (!menuOpen) return;
    menuOpen = false;
    el.menu.classList.remove('show');
    syncCard();
    if (!aboutOpen) invoke('set_menu_open', { open: false });
  }

  function closeAbout() {
    if (!aboutOpen) return;
    aboutOpen = false;
    el.about.classList.remove('show');
    syncCard();
    invoke('set_menu_open', { open: menuOpen });
  }

  el.menu.addEventListener('mousedown', (e) => e.stopPropagation());
  el.about.addEventListener('mousedown', (e) => e.stopPropagation());

  el.menu.addEventListener('click', (e) => {
    const item = e.target.closest('.item');
    if (!item) return;
    const act = item.dataset.act;
    closeMenu();
    if (act === 'pause') {
      const next = !tgt.paused;
      invoke('set_paused', { paused: next });
      toast(next ? '观察已暂停' : '继续观察');
    } else if (act === 'diag') {
      applyDiag(!diagOpen, false);
    } else if (act === 'collect') {
      if (!tgt.can_collect) {
        toast('它还没有定形,再陪它一阵');
        return;
      }
      invoke('collect_creature')
        .then((r) => {
          Scene.burst(40);
          toast(`已收集:${r.name} · ${r.species}`);
        })
        .catch((e) => toast(String(e)));
    } else if (act === 'codex') {
      openCodex();
    } else if (act === 'scale') {
      const scales = [0.8, 1.0, 1.25];
      const i = scales.findIndex((v) => Math.abs(v - uiScale) < 0.01);
      invoke('set_ui_scale', { scale: scales[(i + 1) % scales.length] });
    } else if (act === 'opacity') {
      const values = [1, 0.9, 0.8, 0.7];
      const i = values.findIndex((v) => Math.abs(v - uiOpacity) < 0.01);
      invoke('set_opacity', { value: values[(i + 1) % values.length] });
    } else if (act === 'collapse') {
      invoke('set_collapsed', { value: true });
    } else if (act === 'reset') {
      invoke('reset_jar');
      toast('生态缸已重置');
    } else if (act === 'autostart') {
      const enable = (tgt.autostart_state || 0) !== 1;
      invoke('set_autostart', { enabled: enable }).then((st) => {
        toast(['开机自启已关闭', '开机自启已开启', '被安全软件拦截'][st] || '');
      });
    } else if (act === 'about') {
      applyDiag(false, false);
      aboutOpen = true;
      el.about.classList.add('show');
      syncCard();
      invoke('set_menu_open', { open: true });
    } else if (act === 'quit') {
      invoke('quit_app');
    }
  });

  document.addEventListener('mousedown', (e) => {
    if (menuOpen && !el.menu.contains(e.target)) closeMenu();
    if (aboutOpen && !el.about.contains(e.target)) closeAbout();
    if (diagOpen && !el.diag.contains(e.target)) applyDiag(false, false);
    if (codexOpen && !el.codex.contains(e.target)) closeCodex();
  });

  document.getElementById('diag-close').addEventListener('mousedown', (e) => {
    e.stopPropagation();
    applyDiag(false, false);
  });

  document.getElementById('dock').addEventListener('mousedown', (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.button === 0) invoke('set_collapsed', { value: false });
  });

  document.getElementById('dock').addEventListener('contextmenu', (e) => {
    e.preventDefault();
    e.stopPropagation();
    invoke('set_collapsed', { value: false });
  });

  document.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (codexOpen) {
      closeCodex();
      return;
    }
    if (diagOpen) {
      applyDiag(false, false);
      return;
    }
    if (aboutOpen) {
      closeAbout();
      return;
    }
    openMenu(e.clientX, e.clientY);
  });

  /* ---------- 拖动 ---------- */

  const stage = document.getElementById('stage');
  stage.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    if (menuOpen || aboutOpen || diagOpen) return;
    Scene.react();
    invoke('begin_drag');
  });

  /* ---------- 提示 ---------- */

  function toast(text) {
    el.toast.textContent = text;
    el.toast.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.toast.classList.remove('show'), 1900);
  }

  /* ---------- 缓动与绘制 ---------- */

  function ease(dt) {
    const k = 1 - Math.exp(-dt * 2.2);
    for (const key of Object.keys(cur.traits)) {
      cur.traits[key] += (tgt.traits[key] - cur.traits[key]) * k;
    }
    for (const key of Object.keys(cur.sig)) {
      cur.sig[key] += (tgt.sig[key] - cur.sig[key]) * k;
    }
    cur.growth += (tgt.growth - cur.growth) * k;
    cur.stage = tgt.stage;
    cur.stage_name = tgt.stage_name;
    cur.paused = tgt.paused;
    cur.caption = tgt.caption;
    cur.active_minutes = tgt.active_minutes;
  }

  function fmtMinutes(m) {
    if (m >= 60) return `累计 ${(m / 60).toFixed(1)} 小时`;
    return `累计 ${Math.round(m)} 分钟`;
  }

  function updateCard(now) {
    if (now - lastCard < 160) return;
    lastCard = now;
    el.stageName.textContent = cur.stage_name;
    el.stageMeta.textContent = fmtMinutes(cur.active_minutes);
    el.caption.textContent = tgt.caption;
    el.today.textContent = `今天 ${Math.round(tgt.day_active_min || 0)} 分钟 · ${tgt.day_breaks || 0} 次休息`;
    el.bars.focus.style.width = Math.round(cur.sig.focus * 100) + '%';
    el.bars.rhythm.style.width = Math.round(cur.sig.rhythm * 100) + '%';
    el.bars.calm.style.width = Math.round(cur.sig.calm * 100) + '%';
    el.bars.load.style.width = Math.round(cur.sig.load * 100) + '%';
    el.bars.variety.style.width = Math.round(cur.sig.variety * 100) + '%';
    // 只有一个前台窗口时探索度不可测，条形压暗，避免被当成真实读数。
    el.bars.variety.style.opacity = cur.sig.variety_known ? 1 : 0.4;
  }

  function loop(ts) {
    const dt = lastTs ? Math.min(0.12, (ts - lastTs) / 1000) : 1 / 60;
    lastTs = ts;
    ease(dt);

    // 活跃时 24fps，挂机时降到 8fps：常驻桌面，帧率也要会休息。
    // 探视期间固定 24fps,让老生物游得流畅。
    const previewOn = previewSpec && ts < previewUntil;
    if (!previewOn && previewSpec) previewSpec = null;
    const fps = previewOn ? 24 : tgt.paused ? 10 : tgt.active ? 24 : 8;
    if (!uiCollapsed && ts - lastDraw >= 1000 / fps) {
      dtDraw = Math.max(0.016, (ts - lastDraw) / 1000);
      lastDraw = ts;
      const st = previewOn
        ? previewSpec
        : { ...cur, genome: uiGenome || undefined, archetype: uiArch, species_id: uiSpeciesId };
      Scene.frame(ts, st, dtDraw);
    }
    updateCard(ts);
    requestAnimationFrame(loop);
  }
  requestAnimationFrame(loop);

  /* ---------- 浏览器预览（无 Tauri 时） ---------- */

  function previewMode() {
    // 仅用于在浏览器里检查视觉，不参与真实运行。
    const demo = [
      { focus: 0.82, rhythm: 0.55, calm: 0.72, load: 0.12, variety: 0.3, glow: 0.78, turbidity: 0.16, hue: 0.2, motility: 0.7 },
      { focus: 0.3, rhythm: 0.8, calm: 0.25, load: 0.35, variety: 0.8, glow: 0.4, turbidity: 0.45, hue: 0.55, motility: 0.85 },
      { focus: 0.45, rhythm: 0.3, calm: 0.4, load: 0.75, variety: 0.4, glow: 0.35, turbidity: 0.7, hue: 0.35, motility: 0.5 },
    ];
    let i = 0;
    const pick = () => {
      const d = demo[i % demo.length];
      i++;
      applySnapshot({
        stage: i % 4,
        stage_name: ['孢子', '幼体', '成体', '繁盛'][i % 4],
        growth: (i % 5) / 5,
        paused: false,
        demo: true,
        caption: '（浏览器预览）它在自己找节奏。',
        active_minutes: 120,
        last_break_ms: 0,
        traits: { vitality: 0.7, spots: d.load * 0.6, ...d },
        sig: d,
      });
    };
    pick();
    setInterval(pick, 8000);
    hover = true;
    syncCard();
  }
})();

const invoke = window.__TAURI__.core.invoke;
const api = {
  diagnose: () => invoke('diagnose'),
  selectCodexHome: (currentPath) => invoke('select_codex_home', { currentPath }),
  selectProjectPath: () => invoke('select_project_path'),
  selectPackage: () => invoke('select_package'),
  selectExportPath: (defaultName) => invoke('select_export_path', { defaultName }),
  loadProjects: (codexHome) => invoke('load_projects', { codexHome }),
  loadSessions: (codexHome, cwd) => invoke('load_sessions', { codexHome, cwd }),
  exportSessions: (options) => invoke('export_sessions', { options }),
  inspectPackage: (packagePath) => invoke('inspect_package', { packagePath }),
  importSessions: (options) => invoke('import_sessions', { options }),
  openPath: (filePath) => invoke('open_path', { filePath })
};

const state = {
  mode: 'export',
  codexHome: '',
  projects: [],
  selectedProjectCwd: '',
  sessions: [],
  selectedSessionIds: new Set(),
  packageInfo: null,
  importSelectedIds: new Set()
};

const els = {
  codexHomeInput: document.getElementById('codexHomeInput'),
  chooseCodexHomeBtn: document.getElementById('chooseCodexHomeBtn'),
  refreshBtn: document.getElementById('refreshBtn'),
  exportTab: document.getElementById('exportTab'),
  importTab: document.getElementById('importTab'),
  exportView: document.getElementById('exportView'),
  importView: document.getElementById('importView'),
  notice: document.getElementById('notice'),
  projectSearch: document.getElementById('projectSearch'),
  projectList: document.getElementById('projectList'),
  projectCount: document.getElementById('projectCount'),
  selectedProjectTitle: document.getElementById('selectedProjectTitle'),
  sessionCount: document.getElementById('sessionCount'),
  sessionSearch: document.getElementById('sessionSearch'),
  sessionList: document.getElementById('sessionList'),
  selectAllSessionsBtn: document.getElementById('selectAllSessionsBtn'),
  clearSessionsBtn: document.getElementById('clearSessionsBtn'),
  exportProjectBtn: document.getElementById('exportProjectBtn'),
  exportSelectedBtn: document.getElementById('exportSelectedBtn'),
  packagePathInput: document.getElementById('packagePathInput'),
  choosePackageBtn: document.getElementById('choosePackageBtn'),
  targetProjectInput: document.getElementById('targetProjectInput'),
  chooseTargetProjectBtn: document.getElementById('chooseTargetProjectBtn'),
  addWorkspaceRootInput: document.getElementById('addWorkspaceRootInput'),
  overwriteFilesInput: document.getElementById('overwriteFilesInput'),
  importSelectedBtn: document.getElementById('importSelectedBtn'),
  packageTitle: document.getElementById('packageTitle'),
  packageSummary: document.getElementById('packageSummary'),
  importSessionSearch: document.getElementById('importSessionSearch'),
  selectAllImportBtn: document.getElementById('selectAllImportBtn'),
  clearImportBtn: document.getElementById('clearImportBtn'),
  importSessionList: document.getElementById('importSessionList')
};

document.addEventListener('DOMContentLoaded', init);

async function init() {
  wireEvents();

  try {
    const diagnosis = await api.diagnose();
    state.codexHome = diagnosis.defaultCodexHome || '';
    els.codexHomeInput.value = state.codexHome;

    if (!diagnosis.sqliteAvailable) {
      showNotice(`SQLite 能力不可用：${diagnosis.sqliteError}`, 'error');
      return;
    }

    await refreshProjects();
  } catch (error) {
    showNotice(error.message, 'error');
  }
}

function wireEvents() {
  els.chooseCodexHomeBtn.addEventListener('click', chooseCodexHome);
  els.refreshBtn.addEventListener('click', refreshProjects);
  els.codexHomeInput.addEventListener('change', refreshProjects);

  els.exportTab.addEventListener('click', () => switchMode('export'));
  els.importTab.addEventListener('click', () => switchMode('import'));

  els.projectSearch.addEventListener('input', renderProjects);
  els.sessionSearch.addEventListener('input', renderSessions);
  els.importSessionSearch.addEventListener('input', renderImportSessions);

  els.selectAllSessionsBtn.addEventListener('click', () => {
    state.sessions.filter((session) => session.exists).forEach((session) => state.selectedSessionIds.add(session.id));
    renderSessions();
  });

  els.clearSessionsBtn.addEventListener('click', () => {
    state.selectedSessionIds.clear();
    renderSessions();
  });

  els.exportSelectedBtn.addEventListener('click', () => exportChosenSessions(Array.from(state.selectedSessionIds)));
  els.exportProjectBtn.addEventListener('click', exportProjectSessions);

  els.choosePackageBtn.addEventListener('click', choosePackage);
  els.chooseTargetProjectBtn.addEventListener('click', chooseTargetProject);
  els.selectAllImportBtn.addEventListener('click', () => {
    if (!state.packageInfo) return;
    state.packageInfo.sessions.forEach((session) => state.importSelectedIds.add(session.id));
    renderImportSessions();
  });

  els.clearImportBtn.addEventListener('click', () => {
    state.importSelectedIds.clear();
    renderImportSessions();
  });

  els.importSelectedBtn.addEventListener('click', importChosenSessions);
}

async function chooseCodexHome() {
  const selected = await api.selectCodexHome(els.codexHomeInput.value.trim());
  if (!selected) return;
  els.codexHomeInput.value = selected;
  await refreshProjects();
}

async function refreshProjects() {
  const codexHome = els.codexHomeInput.value.trim();
  if (!codexHome) return;

  setBusy(true);
  try {
    const result = await api.loadProjects(codexHome);
    state.codexHome = result.codexHome;
    els.codexHomeInput.value = result.codexHome;
    state.projects = result.projects || [];
    els.projectCount.textContent = `${state.projects.length} 个项目`;

    const stillExists = state.projects.some((project) => project.cwd === state.selectedProjectCwd);
    state.selectedProjectCwd = stillExists ? state.selectedProjectCwd : (state.projects[0]?.cwd || '');

    renderProjects();
    if (state.selectedProjectCwd) {
      await loadProjectSessions(state.selectedProjectCwd);
      showNotice(`已读取 ${state.projects.length} 个项目。`, 'success');
    } else {
      state.sessions = [];
      state.selectedSessionIds.clear();
      renderSessions();
      showNotice('没有找到 Codex 会话项目。', 'warning');
    }
  } catch (error) {
    showNotice(error.message, 'error');
  } finally {
    setBusy(false);
  }
}

async function loadProjectSessions(cwd) {
  setBusy(true);
  try {
    state.selectedProjectCwd = cwd;
    state.sessions = await api.loadSessions(state.codexHome, cwd);
    state.selectedSessionIds.clear();
    renderProjects();
    renderSessions();
  } catch (error) {
    showNotice(error.message, 'error');
  } finally {
    setBusy(false);
  }
}

function renderProjects() {
  const filter = normalizeSearch(els.projectSearch.value);
  const projects = state.projects.filter((project) => {
    const haystack = normalizeSearch(`${project.name} ${project.displayCwd}`);
    return haystack.includes(filter);
  });

  replaceChildren(els.projectList);

  if (projects.length === 0) {
    els.projectList.className = 'project-list empty-state';
    els.projectList.textContent = '没有匹配的项目。';
    return;
  }

  els.projectList.className = 'project-list';
  for (const project of projects) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `project-item${project.cwd === state.selectedProjectCwd ? ' active' : ''}`;
    button.addEventListener('click', () => loadProjectSessions(project.cwd));

    const title = document.createElement('strong');
    title.textContent = project.name;
    const pathLine = document.createElement('span');
    pathLine.textContent = project.displayCwd;
    const meta = document.createElement('span');
    meta.textContent = `${project.sessionCount} 条会话 · ${project.updatedText || '未知时间'}`;

    button.append(title, pathLine, meta);
    els.projectList.append(button);
  }
}

function renderSessions() {
  const project = state.projects.find((item) => item.cwd === state.selectedProjectCwd);
  els.selectedProjectTitle.textContent = project ? project.name : '选择一个项目';

  const filter = normalizeSearch(els.sessionSearch.value);
  const sessions = state.sessions.filter((session) => {
    const haystack = normalizeSearch(`${session.title} ${session.firstUserMessage} ${session.id}`);
    return haystack.includes(filter);
  });

  const selectedCount = state.selectedSessionIds.size;
  els.sessionCount.textContent = `${state.sessions.length} 条会话 · 已选 ${selectedCount}`;
  els.exportSelectedBtn.disabled = selectedCount === 0;
  els.exportProjectBtn.disabled = state.sessions.length === 0;
  els.selectAllSessionsBtn.disabled = state.sessions.length === 0;
  els.clearSessionsBtn.disabled = selectedCount === 0;

  replaceChildren(els.sessionList);

  if (!state.selectedProjectCwd) {
    setEmpty(els.sessionList, '请选择左侧项目。');
    return;
  }

  if (sessions.length === 0) {
    setEmpty(els.sessionList, '没有匹配的会话。');
    return;
  }

  els.sessionList.className = 'session-list';
  for (const session of sessions) {
    els.sessionList.append(createSessionRow(session, {
      checked: state.selectedSessionIds.has(session.id),
      disabled: !session.exists,
      onChange: (checked) => {
        if (checked) state.selectedSessionIds.add(session.id);
        else state.selectedSessionIds.delete(session.id);
        renderSessions();
      }
    }));
  }
}

async function exportProjectSessions() {
  const exportable = state.sessions.filter((session) => session.exists).map((session) => session.id);
  const missing = state.sessions.length - exportable.length;
  if (missing > 0) {
    showNotice(`有 ${missing} 条会话找不到 jsonl 文件，导出整个项目时会跳过。`, 'warning');
  }
  await exportChosenSessions(exportable);
}

async function exportChosenSessions(sessionIds) {
  const ids = sessionIds.filter(Boolean);
  if (ids.length === 0) {
    showNotice('没有可导出的会话。', 'warning');
    return;
  }

  const project = state.projects.find((item) => item.cwd === state.selectedProjectCwd);
  const defaultName = `${safeFileName(project?.name || 'codex-sessions')}-${ids.length}-${dateForName()}.codexpack`;
  const exportPath = await api.selectExportPath(defaultName);
  if (!exportPath) return;

  setBusy(true);
  try {
    const result = await api.exportSessions({
      codexHome: state.codexHome,
      sessionIds: ids,
      exportPath
    });
    showNotice(`已导出 ${result.sessionCount} 条会话到：${result.exportPath}`, 'success');
  } catch (error) {
    showNotice(error.message, 'error');
  } finally {
    setBusy(false);
  }
}

async function choosePackage() {
  const selected = await api.selectPackage();
  if (!selected) return;
  els.packagePathInput.value = selected;
  await inspectSelectedPackage();
}

async function inspectSelectedPackage() {
  const packagePath = els.packagePathInput.value.trim();
  if (!packagePath) return;

  setBusy(true);
  try {
    const info = await api.inspectPackage(packagePath);
    state.packageInfo = info;
    state.importSelectedIds = new Set(info.sessions.map((session) => session.id));
    renderImportPackage();
    showNotice(`已读取迁移包：${info.sessionCount} 条会话。`, 'success');
  } catch (error) {
    state.packageInfo = null;
    state.importSelectedIds.clear();
    renderImportPackage();
    showNotice(error.message, 'error');
  } finally {
    setBusy(false);
  }
}

async function chooseTargetProject() {
  const selected = await api.selectProjectPath();
  if (!selected) return;
  els.targetProjectInput.value = selected;
}

function renderImportPackage() {
  if (!state.packageInfo) {
    els.packageTitle.textContent = '未选择迁移包';
    els.packageSummary.textContent = '选择迁移包后可勾选导入会话。';
  } else {
    const projects = state.packageInfo.projects || [];
    const projectText = projects.length === 1 ? projects[0].displayCwd : `${projects.length} 个项目`;
    els.packageTitle.textContent = `${state.packageInfo.sessionCount} 条会话`;
    els.packageSummary.textContent = `${projectText} · 导出时间 ${state.packageInfo.exportedAt || '未知'}`;
  }
  renderImportSessions();
}

function renderImportSessions() {
  const selectedCount = state.importSelectedIds.size;
  els.importSelectedBtn.disabled = selectedCount === 0 || !state.packageInfo;
  els.selectAllImportBtn.disabled = !state.packageInfo;
  els.clearImportBtn.disabled = selectedCount === 0;

  replaceChildren(els.importSessionList);
  if (!state.packageInfo) {
    setEmpty(els.importSessionList, '暂无迁移包。');
    return;
  }

  const filter = normalizeSearch(els.importSessionSearch.value);
  const sessions = state.packageInfo.sessions.filter((session) => {
    const haystack = normalizeSearch(`${session.title} ${session.firstUserMessage} ${session.displayCwd} ${session.id}`);
    return haystack.includes(filter);
  });

  if (sessions.length === 0) {
    setEmpty(els.importSessionList, '没有匹配的会话。');
    return;
  }

  els.importSessionList.className = 'session-list';
  for (const session of sessions) {
    els.importSessionList.append(createSessionRow({
      ...session,
      exists: true,
      fileSize: 0,
      model: '',
      createdText: '',
      displayRolloutPath: session.displayCwd
    }, {
      checked: state.importSelectedIds.has(session.id),
      disabled: false,
      onChange: (checked) => {
        if (checked) state.importSelectedIds.add(session.id);
        else state.importSelectedIds.delete(session.id);
        renderImportSessions();
      }
    }));
  }
}

async function importChosenSessions() {
  if (!state.packageInfo) {
    showNotice('请先选择迁移包。', 'warning');
    return;
  }

  const ids = Array.from(state.importSelectedIds);
  if (ids.length === 0) {
    showNotice('至少选择一条会话。', 'warning');
    return;
  }

  setBusy(true);
  try {
    const result = await api.importSessions({
      codexHome: state.codexHome,
      packagePath: state.packageInfo.packagePath,
      sessionIds: ids,
      targetCwd: els.targetProjectInput.value.trim(),
      addWorkspaceRoot: els.addWorkspaceRootInput.checked,
      overwriteFiles: els.overwriteFilesInput.checked
    });
    const message = `已导入 ${result.importedCount} 条会话。导入前备份在：${result.backupDir}`;
    await refreshProjects();
    showNotice(message, 'success');
  } catch (error) {
    showNotice(error.message, 'error');
  } finally {
    setBusy(false);
  }
}

function createSessionRow(session, options) {
  const row = document.createElement('label');
  row.className = 'session-row';

  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  checkbox.checked = options.checked;
  checkbox.disabled = options.disabled;
  checkbox.addEventListener('change', () => options.onChange(checkbox.checked));

  const main = document.createElement('div');
  main.className = 'session-main';

  const title = document.createElement('span');
  title.className = 'session-title';
  title.textContent = session.title || session.id;

  const preview = document.createElement('div');
  preview.className = 'session-preview';
  preview.textContent = brief(session.firstUserMessage) || session.id;

  main.append(title, preview);

  const meta = document.createElement('div');
  meta.className = 'session-meta';
  meta.append(createTag(session.updatedText || '未知时间'));
  if (session.model) meta.append(createTag(session.model));
  if (session.archived) meta.append(createTag('已归档'));
  if (session.fileSize) meta.append(createTag(formatBytes(session.fileSize)));
  if (!session.exists) meta.append(createTag('缺少 jsonl', 'warn'));

  row.append(checkbox, main, meta);
  return row;
}

function createTag(text, kind) {
  const tag = document.createElement('span');
  tag.className = `tag${kind ? ` ${kind}` : ''}`;
  tag.textContent = text;
  return tag;
}

function switchMode(mode) {
  state.mode = mode;
  els.exportTab.classList.toggle('active', mode === 'export');
  els.importTab.classList.toggle('active', mode === 'import');
  els.exportView.classList.toggle('active', mode === 'export');
  els.importView.classList.toggle('active', mode === 'import');
}

function showNotice(message, type = '') {
  els.notice.className = `notice${type ? ` ${type}` : ''}`;
  els.notice.textContent = message;
  if (!message) els.notice.classList.add('hidden');
}

function setBusy(isBusy) {
  document.body.classList.toggle('busy', isBusy);
  const buttons = document.querySelectorAll('button');
  buttons.forEach((button) => {
    button.disabled = isBusy;
  });
  if (!isBusy) {
    renderSessions();
    renderImportSessions();
  }
}

function setEmpty(element, text) {
  element.className = 'session-list empty-state';
  element.textContent = text;
}

function replaceChildren(element) {
  while (element.firstChild) element.removeChild(element.firstChild);
}

function normalizeSearch(value) {
  return String(value || '').toLowerCase().replace(/\s+/g, ' ').trim();
}

function brief(value) {
  return String(value || '').replace(/\s+/g, ' ').trim();
}

function safeFileName(value) {
  return String(value || 'codex-sessions')
    .replace(/[\\/:*?"<>|]+/g, '-')
    .replace(/\s+/g, '-')
    .slice(0, 80);
}

function dateForName() {
  const now = new Date();
  const pad = (value) => String(value).padStart(2, '0');
  return `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
}

function formatBytes(bytes) {
  const value = Number(bytes || 0);
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

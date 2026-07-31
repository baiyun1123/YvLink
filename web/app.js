(() => {
  "use strict";

  const state = {
    token: sessionStorage.getItem("mc_proxy_admin_token") || "",
    status: null,
    config: null,
    crossplay: null,
    lastTraffic: null,
    samples: [],
    chartPaused: false,
    pollTimer: null,
    crossplayTimer: null,
    editingRuleId: null,
    modalReturnFocus: null,
  };

  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];

  async function api(path, options = {}) {
    const headers = new Headers(options.headers || {});
    headers.set("Authorization", `Bearer ${state.token}`);
    if (options.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
    const response = await fetch(`/api/v1${path}`, { ...options, headers });
    const payload = await response.json().catch(() => ({ ok: false, error: { message: `HTTP ${response.status}` } }));
    if (response.status === 401) {
      showAuth();
      throw new Error("管理令牌无效或已失效");
    }
    if (!response.ok || !payload.ok) throw new Error(payload.error?.message || `请求失败：${response.status}`);
    return payload.data;
  }

  function showAuth() {
    clearInterval(state.pollTimer);
    clearInterval(state.crossplayTimer);
    state.pollTimer = null;
    state.crossplayTimer = null;
    $("#authScreen").classList.remove("hidden");
    $("#tokenInput").focus();
  }

  function hideAuth() {
    $("#authScreen").classList.add("hidden");
  }

  async function login(token) {
    state.token = token.trim();
    await api("/session");
    sessionStorage.setItem("mc_proxy_admin_token", state.token);
    hideAuth();
    await bootstrap();
  }

  async function bootstrap() {
    try {
      state.config = await api("/config");
      renderSettings();
      renderCrossplayRouteOptions();
      await Promise.all([pollStatus(), pollCrossplay()]);
      if (!state.pollTimer) state.pollTimer = setInterval(pollStatus, 2000);
      if (!state.crossplayTimer) state.crossplayTimer = setInterval(pollCrossplay, 10000);
    } catch (error) {
      if (!$("#authScreen").classList.contains("hidden")) return;
      toast(error.message, true);
    }
  }

  async function pollStatus() {
    try {
      const next = await api("/status");
      updateTraffic(next.totals);
      state.status = next;
      renderOverview();
      renderRules();
      setConnectionState(true);
    } catch (error) {
      setConnectionState(false);
      if (!error.message.includes("令牌")) toast(error.message, true);
    }
  }

  async function pollCrossplay() {
    try {
      state.crossplay = await api("/crossplay");
      renderCrossplay();
    } catch (error) {
      if (!error.message.includes("令牌")) {
        $("#crossplayMessage").textContent = error.message;
        $("#crossplayMessage").className = "crossplay-message error";
      }
    }
  }

  function updateTraffic(totals) {
    const now = performance.now();
    let uploadRate = 0;
    let downloadRate = 0;
    if (state.lastTraffic) {
      const seconds = Math.max((now - state.lastTraffic.time) / 1000, 0.001);
      uploadRate = Math.max(0, (totals.upload_bytes - state.lastTraffic.upload) / seconds);
      downloadRate = Math.max(0, (totals.download_bytes - state.lastTraffic.download) / seconds);
    }
    state.lastTraffic = { time: now, upload: totals.upload_bytes, download: totals.download_bytes };
    $("#metricUploadRate").textContent = `${formatBytes(uploadRate)}/s`;
    $("#metricDownloadRate").textContent = `${formatBytes(downloadRate)}/s`;
    if (!state.chartPaused) {
      state.samples.push({ upload: uploadRate, download: downloadRate });
      if (state.samples.length > 30) state.samples.shift();
      drawChart();
    }
  }

  function renderOverview() {
    if (!state.status) return;
    const { totals, rules, version, uptime_seconds: uptime } = state.status;
    $("#metricActive").textContent = formatNumber(totals.active_connections);
    $("#metricAccepted").textContent = `累计连接 ${formatNumber(totals.accepted_connections)}`;
    $("#metricUpload").textContent = formatBytes(totals.upload_bytes);
    $("#metricDownload").textContent = formatBytes(totals.download_bytes);
    $("#metricRules").textContent = `${rules.filter(rule => rule.running).length} / ${rules.length}`;
    $("#metricFailures").textContent = `失败 ${formatNumber(totals.backend_failures + totals.forwarding_failures)}`;
    $("#instanceVersion").textContent = `v${version}`;
    $("#instanceUptime").textContent = formatUptime(uptime);
    $("#instanceBackendFailures").textContent = formatNumber(totals.backend_failures);
    $("#instanceForwardFailures").textContent = formatNumber(totals.forwarding_failures);
    $("#instanceRejected").textContent = formatNumber(totals.rejected_connections);
    $("#instanceWhitelistDenials").textContent = formatNumber(totals.whitelist_denials);
    $("#instanceLocalStatus").textContent = formatNumber(totals.local_status_responses);
    $("#instanceStatusCacheHits").textContent = formatNumber(totals.status_cache_hits);
    $("#instanceStatusFallbacks").textContent = formatNumber(totals.status_fallbacks);
    $("#instanceBackendAttemptFailures").textContent = formatNumber(totals.backend_attempt_failures);
    $("#instanceBackendFailovers").textContent = formatNumber(totals.backend_failovers);
    $("#instanceProxyV1Headers").textContent = formatNumber(totals.proxy_protocol_v1_headers);
    $("#instanceProxyV2Headers").textContent = formatNumber(totals.proxy_protocol_v2_headers);
    $("#instanceHealthCheckSuccesses").textContent = formatNumber(totals.health_check_successes);
    $("#instanceHealthCheckFailures").textContent = formatNumber(totals.health_check_failures);
    $("#protocolUnmarked").textContent = formatNumber(totals.unmarked_handshakes);
    $("#protocolLegacyForge").textContent = formatNumber(totals.legacy_forge_handshakes);
    $("#protocolModernForgeLogin").textContent = formatNumber(totals.modern_forge_login_handshakes);
    $("#protocolConfigurationForge").textContent = formatNumber(totals.configuration_forge_handshakes);
    $("#ingressAddress").textContent = state.config.settings.listen;
    $("#proxyStateChip").classList.toggle("online", Boolean(state.status.proxy_running));
    $("#proxyStateText").textContent = state.status.proxy_running ? "入口运行中" : "入口已停用";

    const rows = $("#overviewRuleRows");
    rows.replaceChildren();
    if (!rules.length) {
      rows.innerHTML = '<tr><td colspan="5" class="empty-cell">暂无转发规则</td></tr>';
      return;
    }
    for (const rule of rules) {
      const row = document.createElement("tr");
      row.innerHTML = `
        <td><strong>${escapeHtml(rule.name)}</strong><span>${escapeHtml(rule.id)}</span></td>
        <td>${rule.host.map(escapeHtml).join(", ")}</td>
        <td>${backendList(rule).map(escapeHtml).join("<br>")}</td>
        <td>${escapeHtml(state.config.settings.listen)}</td>
        <td><span class="state-badge ${rule.running ? "" : "off"}">${rule.running ? "RUNNING" : "STOPPED"}</span></td>`;
      rows.append(row);
    }
  }

  function renderRules() {
    if (!state.status) return;
    const grid = $("#ruleGrid");
    grid.replaceChildren();
    for (const rule of state.status.rules) {
      const health = rule.backend_health || [];
      const activeBackends = health.reduce((sum, backend) => sum + Number(backend.active_connections || 0), 0);
      const failedAttempts = health.reduce((sum, backend) => sum + Number(backend.failed_attempts || 0), 0);
      const backendHealthRows = health.map(backend => {
        const stateView = backendHealthView(backend.health, rule.health_check?.enabled);
        const checked = backend.last_checked_secs_ago == null ? "尚未检查" : `${formatNumber(backend.last_checked_secs_ago)} 秒前`;
        const latency = backend.health_check_latency_ms == null ? "--" : `${formatNumber(backend.health_check_latency_ms)} ms`;
        const streak = backend.health === "unhealthy"
          ? `连续失败 ${formatNumber(backend.consecutive_health_failures)}`
          : `连续成功 ${formatNumber(backend.consecutive_health_successes)}`;
        return `<div class="backend-health-row">
          <div><strong>${escapeHtml(backend.address)}</strong><small>${checked} · 探测 ${latency} · ${streak}</small></div>
          <span class="health-badge ${stateView.className}"><i aria-hidden="true"></i>${stateView.label}</span>
        </div>`;
      }).join("");
      const card = document.createElement("article");
      card.className = "rule-card";
      card.innerHTML = `
        <div class="rule-card-head">
          <div><h2>${escapeHtml(rule.name)}</h2><span class="rule-card-id">${escapeHtml(rule.id)}</span></div>
          <span class="state-badge ${rule.running ? "" : "off"}">${rule.running ? "RUNNING" : "STOPPED"}</span>
        </div>
        <div class="rule-route">
          <div class="route-node"><span>Host</span><strong>${rule.host.map(escapeHtml).join(", ")}</strong></div>
          <div class="route-arrow"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14m-5-5 5 5-5 5"/></svg></div>
          <div class="route-node"><span>Backend Pool</span><strong>${backendList(rule).map(escapeHtml).join(" · ")}</strong></div>
        </div>
        <div class="rule-metrics">
          <div><span>共享入口</span><strong>${escapeHtml(state.config.settings.listen)}</strong></div>
          <div><span>负载策略</span><strong>${strategyLabel(rule.strategy)}</strong></div>
          <div><span>真实 IP 传递</span><strong>${proxyProtocolLabel(rule.proxy_protocol)}</strong></div>
          <div><span>活动后端连接</span><strong>${formatNumber(activeBackends)}</strong></div>
          <div><span>后端尝试失败</span><strong>${formatNumber(failedAttempts)}</strong></div>
          <div><span>状态响应</span><strong>${rule.status ? (rule.status.mode === "backend" ? "后端覆盖 + 缓存" : "代理完全生成") : "透明透传"}</strong></div>
          <div><span>主动探测</span><strong>${!rule.health_check?.enabled ? "关闭" : rule.health_check.mode === "minecraft-status" ? "Minecraft Status" : "TCP 端口"}</strong></div>
          <div><span>访问控制</span><strong>${rule.whitelist_enabled ? `白名单 ${rule.whitelist.length} 人` : "后端负责"}</strong></div>
          <div><span>后端握手 Host</span><strong>${rule.modify_virtual_host ? "改写为后端主机" : "保留客户端域名"}</strong></div>
        </div>
        <section class="backend-health-list" aria-label="${escapeHtml(rule.name)} 后端健康状态">
          ${backendHealthRows || '<p class="backend-health-empty">路由未运行，暂无后端状态。</p>'}
        </section>
        <div class="rule-actions">
          <button class="secondary-button" data-toggle="${escapeHtml(rule.id)}">${rule.enabled ? "停用" : "启用"}</button>
          <button class="secondary-button" data-edit="${escapeHtml(rule.id)}">编辑</button>
          <button class="danger-button" data-delete="${escapeHtml(rule.id)}">删除</button>
        </div>`;
      grid.append(card);
    }
    if (!state.status.rules.length) grid.innerHTML = '<article class="panel empty-cell">暂无转发规则</article>';
  }

  function drawChart() {
    const samples = state.samples;
    const points = samples.length > 1 ? samples : [{ upload: 0, download: 0 }, ...samples];
    const max = Math.max(1, ...points.flatMap(point => [point.upload, point.download]));
    const path = key => points.map((point, index) => {
      const x = points.length === 1 ? 0 : (index / (points.length - 1)) * 800;
      const y = 240 - (point[key] / max) * 210;
      return `${index ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
    }).join(" ");
    const area = line => `${line} L800 240 L0 240 Z`;
    const upload = path("upload");
    const download = path("download");
    $("#uploadLine").setAttribute("d", upload);
    $("#downloadLine").setAttribute("d", download);
    $("#uploadArea").setAttribute("d", area(upload));
    $("#downloadArea").setAttribute("d", area(download));
    $("#chartMax").textContent = `${formatBytes(max)}/s`;
  }

  function renderSettings() {
    if (!state.config) return;
    const form = $("#settingsForm");
    for (const [key, value] of Object.entries(state.config.settings)) {
      const field = form.elements.namedItem(key);
      if (!field) continue;
      if (field.type === "checkbox") field.checked = value;
      else field.value = value;
    }
  }

  function renderCrossplayRouteOptions() {
    const options = $("#crossplayRouteHosts");
    options.replaceChildren();
    for (const rule of state.config?.rules || []) {
      for (const host of rule.host || []) {
        if (host.includes("*") || host.includes("?")) continue;
        const option = document.createElement("option");
        option.value = host;
        option.label = `${rule.name} · ${rule.id}`;
        options.append(option);
      }
    }
  }

  function renderCrossplay() {
    if (!state.crossplay) return;
    const { config, status } = state.crossplay;
    const form = $("#crossplayForm");
    form.elements.enabled.checked = config.enabled;
    form.elements.bedrock_listen.value = config.bedrock_listen;
    form.elements.java_address.value = config.java_address;
    form.elements.java_port.value = config.java_port;
    form.elements.auth_type.value = config.auth_type;
    form.elements.provider.value = config.provider;
    form.elements.geyser_mode.value = config.geyserlite.mode;
    form.elements.geyser_motd_line1.value = config.geyserlite.motd_line1;
    form.elements.geyser_motd_line2.value = config.geyserlite.motd_line2;
    form.elements.geyser_library_path.value = config.geyserlite.library_path || "";
    form.elements.geyser_binary_path.value = config.geyserlite.binary_path || "";
    form.elements.geyser_offline.checked = config.geyserlite.offline;
    form.elements.geyser_floodgate_key.value = config.geyserlite.floodgate_key || "";
    $("#crossplayListen").textContent = status.bedrock_listen;
    $("#crossplayJavaTarget").textContent = status.java_target;
    $("#crossplayAuthType").textContent = crossplayAuthLabel(status.auth_type);
    $("#crossplayProvider").textContent = crossplayProviderLabel(config.provider);
    $("#crossplayRuntime").textContent = crossplayRuntimeLabel(config, state.crossplay.runtime);
    $("#crossplayLatency").textContent = status.latency_ms == null ? "--" : `${status.latency_ms} ms`;
    const badge = $("#crossplayStateBadge");
    badge.classList.toggle("off", !status.online);
    badge.textContent = status.online ? "ONLINE" : status.enabled ? "OFFLINE" : "DISABLED";
    $("#crossplayLiveLabel").classList.toggle("off", !status.online);
    $("#crossplayHealthLabel").textContent = status.online
      ? "UDP 在线"
      : status.enabled
        ? config.provider === "geyserlite" ? "等待 GeyserLite" : "等待 Geyser"
        : "未启用";
    const runtimeError = state.crossplay.runtime?.error || null;
    const message = $("#crossplayMessage");
    message.textContent = status.online
      ? `RakNet Pong 正常${status.motd ? ` · ${status.motd.split(";").slice(0, 2).join(" · ")}` : ""}`
      : status.error || runtimeError || "互通监控未启用；Java 路由不受影响。";
    message.className = `crossplay-message${status.online ? " success" : status.error || runtimeError ? " error" : ""}`;
    syncCrossplayFields();
  }

  function syncCrossplayFields() {
    const form = $("#crossplayForm");
    const provider = form.elements.provider?.value;
    const authType = form.elements.auth_type?.value;
    const mode = form.elements.geyser_mode?.value;
    const geyserliteSection = $("#geyserliteSection");
    const embeddedOnly = form.elements.geyser_library_path?.closest("label");
    const subprocessOnly = form.elements.geyser_binary_path?.closest("label");
    const floodgateOnly = form.elements.geyser_floodgate_key?.closest("label");
    if (geyserliteSection) geyserliteSection.hidden = provider !== "geyserlite";
    if (embeddedOnly) embeddedOnly.hidden = provider !== "geyserlite" || mode !== "embedded";
    if (subprocessOnly) subprocessOnly.hidden = provider !== "geyserlite" || mode !== "subprocess";
    if (floodgateOnly) floodgateOnly.hidden = provider !== "geyserlite" || authType !== "floodgate";
    $("#crossplaySettingsHint").textContent = provider === "geyserlite"
      ? "由 mc-proxy 托管 GeyserLite"
      : "外部 Geyser 必须使用相同参数";
  }

  function openRuleModal(rule = null) {
    state.modalReturnFocus = document.activeElement;
    state.editingRuleId = rule?.id || null;
    $("#ruleModalTitle").textContent = rule ? "编辑路由" : "新建路由";
    const form = $("#ruleForm");
    form.reset();
    form.elements.id.disabled = Boolean(rule);
    if (rule) {
      for (const key of ["id", "name"]) form.elements[key].value = rule[key];
      form.elements.host.value = rule.host.join(", ");
      form.elements.backend.value = backendList(rule).join("\n");
      form.elements.strategy.value = rule.strategy || "sequential";
      form.elements.proxy_protocol.value = rule.proxy_protocol || "off";
      form.elements.health_check_enabled.checked = Boolean(rule.health_check?.enabled);
      form.elements.health_check_mode.value = rule.health_check?.mode || "tcp";
      form.elements.health_check_interval_secs.value = rule.health_check?.interval_secs ?? 30;
      form.elements.health_check_timeout_ms.value = rule.health_check?.timeout_ms ?? 2000;
      form.elements.health_check_unhealthy_threshold.value = rule.health_check?.unhealthy_threshold ?? 3;
      form.elements.health_check_healthy_threshold.value = rule.health_check?.healthy_threshold ?? 2;
      form.elements.health_check_minecraft_host.value = rule.health_check?.minecraft_host ?? "";
      form.elements.health_check_minecraft_protocol.value = rule.health_check?.minecraft_protocol ?? 769;
      form.elements.modify_virtual_host.checked = Boolean(rule.modify_virtual_host);
      form.elements.status_enabled.checked = Boolean(rule.status);
      form.elements.status_mode.value = rule.status?.mode || "custom";
      form.elements.status_cache_ttl_secs.value = rule.status?.cache_ttl_secs ?? 10;
      form.elements.status_motd.value = rule.status?.motd ?? "";
      form.elements.status_version_name.value = rule.status?.version_name ?? "";
      form.elements.status_protocol.value = rule.status?.protocol ?? "";
      form.elements.status_online.value = rule.status?.online ?? "";
      form.elements.status_max.value = rule.status?.max ?? "";
      form.elements.status_fallback_enabled.checked = Boolean(rule.status?.fallback);
      form.elements.fallback_motd.value = rule.status?.fallback?.motd ?? "";
      form.elements.fallback_version_name.value = rule.status?.fallback?.version_name ?? "";
      form.elements.fallback_protocol.value = rule.status?.fallback?.protocol ?? "";
      form.elements.fallback_online.value = rule.status?.fallback?.online ?? "";
      form.elements.fallback_max.value = rule.status?.fallback?.max ?? "";
      form.elements.whitelist_enabled.checked = Boolean(rule.whitelist_enabled);
      form.elements.whitelist.value = (rule.whitelist || []).join("\n");
      form.elements.whitelist_message.value = rule.whitelist_message || "§c你不在此服务器的白名单中。";
      form.elements.enabled.checked = rule.enabled;
    } else {
      form.elements.enabled.checked = true;
      form.elements.backend.value = "127.0.0.1:25566";
      form.elements.strategy.value = "sequential";
      form.elements.proxy_protocol.value = "off";
      form.elements.health_check_enabled.checked = false;
      form.elements.health_check_mode.value = "minecraft-status";
      form.elements.health_check_interval_secs.value = 30;
      form.elements.health_check_timeout_ms.value = 2000;
      form.elements.health_check_unhealthy_threshold.value = 3;
      form.elements.health_check_healthy_threshold.value = 2;
      form.elements.health_check_minecraft_host.value = "";
      form.elements.health_check_minecraft_protocol.value = 769;
      form.elements.host.value = "";
      form.elements.modify_virtual_host.checked = false;
      form.elements.status_enabled.checked = false;
      form.elements.status_mode.value = "custom";
      form.elements.status_cache_ttl_secs.value = 10;
      form.elements.status_motd.value = "§aMinecraft Server";
      form.elements.status_version_name.value = "MC Relay";
      form.elements.status_protocol.value = "";
      form.elements.status_online.value = 0;
      form.elements.status_max.value = 100;
      form.elements.status_fallback_enabled.checked = false;
      form.elements.fallback_motd.value = "§c服务器维护中，请稍后再试";
      form.elements.fallback_version_name.value = "后端离线";
      form.elements.fallback_protocol.value = -1;
      form.elements.fallback_online.value = 0;
      form.elements.fallback_max.value = 100;
      form.elements.whitelist_enabled.checked = false;
      form.elements.whitelist.value = "";
      form.elements.whitelist_message.value = "§c你不在此服务器的白名单中。";
    }
    syncRuleAdvancedFields();
    $("#ruleModal").hidden = false;
    form.elements[rule ? "name" : "id"].focus();
  }

  function closeRuleModal() {
    $("#ruleModal").hidden = true;
    state.editingRuleId = null;
    state.modalReturnFocus?.focus();
    state.modalReturnFocus = null;
  }

  async function saveRule(form) {
    const submit = form.querySelector('[type="submit"]');
    const originalLabel = submit.textContent;
    submit.disabled = true;
    submit.textContent = "正在保存…";
    const statusEnabled = form.elements.status_enabled.checked;
    const optionalNumber = name => {
      const value = form.elements[name].value.trim();
      return value === "" ? null : Number(value);
    };
    const optionalText = name => {
      const value = form.elements[name].value;
      return value.trim() === "" ? null : value;
    };
    const fallbackEnabled = statusEnabled
      && form.elements.status_mode.value === "backend"
      && form.elements.status_fallback_enabled.checked;
    const rule = {
      id: state.editingRuleId || form.elements.id.value.trim(),
      name: form.elements.name.value.trim(),
      host: form.elements.host.value.split(",").map(host => host.trim()).filter(Boolean),
      backend: form.elements.backend.value.split(/[\n,]+/).map(backend => backend.trim()).filter(Boolean),
      strategy: form.elements.strategy.value,
      proxy_protocol: form.elements.proxy_protocol.value,
      health_check: {
        enabled: form.elements.health_check_enabled.checked,
        mode: form.elements.health_check_mode.value,
        interval_secs: Number(form.elements.health_check_interval_secs.value),
        timeout_ms: Number(form.elements.health_check_timeout_ms.value),
        unhealthy_threshold: Number(form.elements.health_check_unhealthy_threshold.value),
        healthy_threshold: Number(form.elements.health_check_healthy_threshold.value),
        minecraft_host: optionalText("health_check_minecraft_host"),
        minecraft_protocol: Number(form.elements.health_check_minecraft_protocol.value),
      },
      modify_virtual_host: form.elements.modify_virtual_host.checked,
      status: statusEnabled ? {
        mode: form.elements.status_mode.value,
        cache_ttl_secs: Number(form.elements.status_cache_ttl_secs.value),
        motd: optionalText("status_motd"),
        version_name: optionalText("status_version_name"),
        protocol: optionalNumber("status_protocol"),
        online: optionalNumber("status_online"),
        max: optionalNumber("status_max"),
        fallback: fallbackEnabled ? {
          motd: optionalText("fallback_motd"),
          version_name: optionalText("fallback_version_name"),
          protocol: optionalNumber("fallback_protocol"),
          online: optionalNumber("fallback_online"),
          max: optionalNumber("fallback_max"),
        } : null,
      } : null,
      whitelist_enabled: form.elements.whitelist_enabled.checked,
      whitelist: form.elements.whitelist.value.split(/[\n,]+/).map(player => player.trim()).filter(Boolean),
      whitelist_message: form.elements.whitelist_message.value,
      enabled: form.elements.enabled.checked,
    };
    const path = state.editingRuleId ? `/rules/${encodeURIComponent(state.editingRuleId)}` : "/rules";
    const method = state.editingRuleId ? "PUT" : "POST";
    try {
      await api(path, { method, body: JSON.stringify(rule) });
      closeRuleModal();
      state.config = await api("/config");
      await pollStatus();
      toast("转发规则已保存并应用");
    } finally {
      submit.disabled = false;
      submit.textContent = originalLabel;
    }
  }

  async function toggleRule(id) {
    const rule = state.status.rules.find(item => item.id === id);
    if (!rule) return;
    const payload = {
      id: rule.id, name: rule.name, host: rule.host, backend: backendList(rule),
      strategy: rule.strategy || "sequential",
      proxy_protocol: rule.proxy_protocol || "off",
      health_check: rule.health_check || {
        enabled: false, mode: "tcp", interval_secs: 30, timeout_ms: 2000,
        unhealthy_threshold: 3, healthy_threshold: 2,
        minecraft_host: null, minecraft_protocol: 769,
      },
      modify_virtual_host: Boolean(rule.modify_virtual_host),
      status: rule.status || null,
      whitelist_enabled: Boolean(rule.whitelist_enabled),
      whitelist: rule.whitelist || [],
      whitelist_message: rule.whitelist_message || "§c你不在此服务器的白名单中。",
      enabled: !rule.enabled,
    };
    await api(`/rules/${encodeURIComponent(id)}`, { method: "PUT", body: JSON.stringify(payload) });
    state.config = await api("/config");
    await pollStatus();
    toast(payload.enabled ? "规则已启用" : "规则已停用");
  }

  async function deleteRule(id) {
    const rule = state.status.rules.find(item => item.id === id);
    if (!rule || !confirm(`确认删除转发规则“${rule.name}”？`)) return;
    await api(`/rules/${encodeURIComponent(id)}`, { method: "DELETE" });
    state.config = await api("/config");
    await pollStatus();
    toast("转发规则已删除");
  }

  async function saveSettings(form) {
    const submit = form.querySelector('[type="submit"]');
    const originalLabel = submit.textContent;
    submit.disabled = true;
    submit.textContent = "正在应用…";
    const payload = {};
    for (const key of [
      "max_connections", "connect_timeout_ms", "handshake_timeout_ms", "shutdown_grace_secs", "copy_buffer_bytes",
      "socket_buffer_bytes", "listen_backlog", "stats_interval_secs",
    ]) payload[key] = Number(form.elements[key].value);
    payload.tcp_nodelay = form.elements.tcp_nodelay.checked;
    payload.reuse_port = form.elements.reuse_port.checked;
    payload.proxy_enabled = form.elements.proxy_enabled.checked;
    payload.listen = form.elements.listen.value.trim();
    try {
      state.config = await api("/config", { method: "PUT", body: JSON.stringify(payload) });
      renderSettings();
      await pollStatus();
      toast("全局配置已保存并应用");
    } finally {
      submit.disabled = false;
      submit.textContent = originalLabel;
    }
  }

  async function saveCrossplay(form) {
    const submit = form.querySelector('[type="submit"]');
    const originalLabel = submit.textContent;
    submit.disabled = true;
    submit.textContent = "正在验证…";
    const payload = {
      enabled: form.elements.enabled.checked,
      provider: form.elements.provider.value,
      bedrock_listen: form.elements.bedrock_listen.value.trim(),
      java_address: form.elements.java_address.value.trim(),
      java_port: Number(form.elements.java_port.value),
      auth_type: form.elements.auth_type.value,
      geyserlite: {
        mode: form.elements.geyser_mode.value,
        library_path: form.elements.geyser_library_path.value.trim() || null,
        binary_path: form.elements.geyser_binary_path.value.trim() || null,
        offline: form.elements.geyser_offline.checked,
        motd_line1: form.elements.geyser_motd_line1.value.trim(),
        motd_line2: form.elements.geyser_motd_line2.value.trim(),
        floodgate_key: form.elements.geyser_floodgate_key.value.trim() || null,
      },
    };
    try {
      state.crossplay = await api("/crossplay", { method: "PUT", body: JSON.stringify(payload) });
      state.config = await api("/config");
      renderCrossplay();
      toast(state.crossplay.status.online ? "互通配置已保存，Geyser UDP 在线" : "互通配置已保存，等待 Geyser 上线");
    } finally {
      submit.disabled = false;
      submit.textContent = originalLabel;
    }
  }

  function navigate(page) {
    $$(".nav-item").forEach(item => {
      const active = item.dataset.page === page;
      item.classList.toggle("active", active);
      item.setAttribute("aria-selected", String(active));
    });
    $$(".page").forEach(item => item.classList.toggle("active", item.id === `page-${page}`));
    const labels = { overview: "运行概览", rules: "域名路由", crossplay: "基岩版互通", settings: "入口配置" };
    $("#pageTitle").textContent = labels[page] || labels.overview;
    $("#sidebar").classList.remove("open");
    $("#mobileMenu").setAttribute("aria-expanded", "false");
  }

  function syncRuleAdvancedFields() {
    const form = $("#ruleForm");
    const statusEnabled = form.elements.status_enabled.checked;
    const backendMode = statusEnabled && form.elements.status_mode.value === "backend";
    const whitelistEnabled = form.elements.whitelist_enabled.checked;
    const healthCheckEnabled = form.elements.health_check_enabled.checked;
    const minecraftHealthEnabled = healthCheckEnabled
      && form.elements.health_check_mode.value === "minecraft-status";
    const fallbackEnabled = backendMode && form.elements.status_fallback_enabled.checked;
    $("#statusFields").hidden = !statusEnabled;
    $("#whitelistFields").hidden = !whitelistEnabled;
    $("#healthCheckFields").hidden = !healthCheckEnabled;
    $("#fallbackFields").hidden = !fallbackEnabled;
    $$(".backend-status-only", form).forEach(item => { item.hidden = !backendMode; });
    $$(".minecraft-health-only", form).forEach(item => { item.hidden = !minecraftHealthEnabled; });
    form.elements.status_enabled.setAttribute("aria-expanded", String(statusEnabled));
    form.elements.health_check_enabled.setAttribute("aria-expanded", String(healthCheckEnabled));
    form.elements.status_fallback_enabled.setAttribute("aria-expanded", String(fallbackEnabled));
    form.elements.whitelist.required = whitelistEnabled;
    form.elements.whitelist_message.required = whitelistEnabled;
    for (const name of ["health_check_mode", "health_check_interval_secs", "health_check_timeout_ms", "health_check_unhealthy_threshold", "health_check_healthy_threshold"]) {
      form.elements[name].required = healthCheckEnabled;
    }
    form.elements.health_check_minecraft_protocol.required = minecraftHealthEnabled;
  }

  function setConnectionState(online) {
    const dot = $("#statusDot");
    dot.className = `status-dot ${online ? "online" : "offline"}`;
    $("#connectionState").textContent = online ? "管理端在线" : "连接中断";
    $("#lastUpdated").textContent = online ? `同步于 ${new Date().toLocaleTimeString("zh-CN", { hour12: false })}` : "等待重新连接";
  }

  function formatBytes(value) {
    const number = Number(value) || 0;
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let result = number;
    let index = 0;
    while (Math.abs(result) >= 1024 && index < units.length - 1) { result /= 1024; index += 1; }
    const digits = index === 0 ? 0 : result >= 100 ? 0 : result >= 10 ? 1 : 2;
    return `${result.toFixed(digits)} ${units[index]}`;
  }

  function formatNumber(value) {
    return new Intl.NumberFormat("zh-CN").format(Number(value) || 0);
  }

  function backendList(rule) {
    return Array.isArray(rule.backend) ? rule.backend : [rule.backend].filter(Boolean);
  }

  function strategyLabel(strategy) {
    return ({
      sequential: "顺序故障转移",
      random: "随机",
      "round-robin": "轮询",
      "least-connections": "最少连接",
      "lowest-latency": "最低延迟",
    })[strategy || "sequential"] || strategy;
  }

  function proxyProtocolLabel(version) {
    return ({ off: "关闭", v1: "PROXY v1", v2: "PROXY v2" })[version || "off"] || version;
  }

  function backendHealthView(health, enabled) {
    if (!enabled) return { className: "disabled", label: "未启用" };
    return ({
      healthy: { className: "healthy", label: "健康" },
      unhealthy: { className: "unhealthy", label: "离线" },
      unknown: { className: "unknown", label: "等待首检" },
    })[health || "unknown"] || { className: "unknown", label: "状态未知" };
  }

  function crossplayAuthLabel(authType) {
    return ({ online: "Online", floodgate: "Floodgate", offline: "Offline" })[authType] || authType;
  }

  function crossplayProviderLabel(provider) {
    return ({ external: "外部 Geyser Standalone", geyserlite: "内置 GeyserLite" })[provider] || provider;
  }

  function crossplayRuntimeLabel(config, runtime) {
    if (!runtime) return "--";
    if (config.provider !== "geyserlite") return "独立进程 · 未托管";
    if (!runtime.available) return "当前平台/构建未启用 GeyserLite";
    if (runtime.running) {
      const mode = runtime.mode === "subprocess" ? "子进程" : "进程内";
      return `托管中 · ${mode}`;
    }
    return runtime.error ? "启动失败" : "已停止";
  }

  function formatUptime(seconds) {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return days ? `${days}天 ${hours}小时` : hours ? `${hours}小时 ${minutes}分` : `${minutes}分`;
  }

  function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, character => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    })[character]);
  }

  function toast(message, error = false) {
    const item = document.createElement("div");
    item.className = `toast${error ? " error" : ""}`;
    item.textContent = message;
    $("#toastStack").append(item);
    setTimeout(() => item.remove(), 4200);
  }

  function toggleChart() {
    state.chartPaused = !state.chartPaused;
    $("#chartToggle").setAttribute("aria-pressed", String(state.chartPaused));
    $("#chartToggleText").textContent = state.chartPaused ? "继续" : "暂停";
    $("#chartToggle").querySelector("path").setAttribute(
      "d",
      state.chartPaused ? "M8 5v14l11-7L8 5Z" : "M8 5v14m8-14v14",
    );
    if (!state.chartPaused) drawChart();
  }

  $("#loginForm").addEventListener("submit", async event => {
    event.preventDefault();
    $("#loginError").textContent = "";
    const submit = event.currentTarget.querySelector('[type="submit"]');
    const originalLabel = submit.textContent;
    submit.disabled = true;
    submit.textContent = "正在验证…";
    try { await login($("#tokenInput").value); }
    catch (error) { $("#loginError").textContent = error.message; }
    finally { submit.disabled = false; submit.textContent = originalLabel; }
  });
  $("#logoutButton").addEventListener("click", () => {
    sessionStorage.removeItem("mc_proxy_admin_token");
    state.token = "";
    state.status = null;
    state.config = null;
    state.crossplay = null;
    showAuth();
  });
  $$(".nav-item").forEach(item => item.addEventListener("click", () => navigate(item.dataset.page)));
  $$("[data-go-page]").forEach(item => item.addEventListener("click", () => navigate(item.dataset.goPage)));
  $("#mobileMenu").addEventListener("click", () => {
    const open = $("#sidebar").classList.toggle("open");
    $("#mobileMenu").setAttribute("aria-expanded", String(open));
  });
  $("#chartToggle").addEventListener("click", toggleChart);
  $("#addRuleButton").addEventListener("click", () => openRuleModal());
  $$("[data-close-modal]").forEach(item => item.addEventListener("click", closeRuleModal));
  $("#ruleModal").addEventListener("click", event => { if (event.target === event.currentTarget) closeRuleModal(); });
  $("#ruleForm").addEventListener("submit", async event => {
    event.preventDefault();
    try { await saveRule(event.currentTarget); } catch (error) { toast(error.message, true); }
  });
  $("#ruleForm").elements.status_enabled.addEventListener("change", syncRuleAdvancedFields);
  $("#ruleForm").elements.health_check_enabled.addEventListener("change", syncRuleAdvancedFields);
  $("#ruleForm").elements.health_check_mode.addEventListener("change", syncRuleAdvancedFields);
  $("#ruleForm").elements.status_mode.addEventListener("change", event => {
    if (event.target.value === "backend") {
      for (const name of ["status_motd", "status_version_name", "status_protocol", "status_online", "status_max"]) {
        event.currentTarget.form.elements[name].value = "";
      }
    }
    syncRuleAdvancedFields();
  });
  $("#ruleForm").elements.status_fallback_enabled.addEventListener("change", syncRuleAdvancedFields);
  $("#ruleForm").elements.whitelist_enabled.addEventListener("change", syncRuleAdvancedFields);
  $("#settingsForm").addEventListener("submit", async event => {
    event.preventDefault();
    try { await saveSettings(event.currentTarget); } catch (error) { toast(error.message, true); }
  });
  $("#crossplayForm").addEventListener("submit", async event => {
    event.preventDefault();
    try { await saveCrossplay(event.currentTarget); } catch (error) { toast(error.message, true); }
  });
  for (const name of ["provider", "auth_type", "geyser_mode"]) {
    $("#crossplayForm").elements[name].addEventListener("change", syncCrossplayFields);
  }
  $("#ruleGrid").addEventListener("click", async event => {
    const edit = event.target.closest("[data-edit]");
    const toggle = event.target.closest("[data-toggle]");
    const remove = event.target.closest("[data-delete]");
    try {
      if (edit) openRuleModal(state.status.rules.find(rule => rule.id === edit.dataset.edit));
      if (toggle) await toggleRule(toggle.dataset.toggle);
      if (remove) await deleteRule(remove.dataset.delete);
    } catch (error) { toast(error.message, true); }
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape" && !$("#ruleModal").hidden) closeRuleModal();
    if (event.key === "Escape" && $("#sidebar").classList.contains("open")) {
      $("#sidebar").classList.remove("open");
      $("#mobileMenu").setAttribute("aria-expanded", "false");
      $("#mobileMenu").focus();
    }
  });

  if (state.token) bootstrap();
  else showAuth();
})();

// Realm Management Web Interface
import { apiCall } from './js/api.js';
import { showModal, hideModal, showSuccess, showError, showLoading, hideLoading, showNotification, addActivity, addLogLine, clearLogs } from './js/utils.js';
import { setupJobsHandlers as setupJobsHandlersModule, loadJobsData as loadJobsDataModule, openJobSubmitModal as openJobSubmitModalModule, viewJob as viewJobModule, cancelJob as cancelJobModule } from './js/jobs.js';
import { populateLogComponents as populateLogComponentsModule, loadLogsData as loadLogsDataModule } from './js/logs.js';

class RealmApp {
    constructor() {
        this.currentView = 'overview';
        this.sessionToken = this.getSessionToken();
        this.websocket = null;
        this.autoScroll = true;
        this.selectedLogComponent = '__all__';
        
        this.init();
    }

    init() {
        this.setupNavigation();
        this.setupEventHandlers();
        this.connectWebSocket();
        this.loadInitialData();
        
        // Refresh data every 30 seconds
        setInterval(() => this.refreshData(), 30000);
    }

    getSessionToken() {
        // Extract session token from URL or cookie
        const urlParams = new URLSearchParams(window.location.search);
        return urlParams.get('token') || 'session-token';
    }

    setupNavigation() {
        this.navItems = Array.from(document.querySelectorAll('.nav-item'));
        this.navItems.forEach(item => {
            item.addEventListener('click', (e) => {
                const view = e.currentTarget.dataset.view;
                if (view) this.switchView(view);
            });
        });
    }

    setupEventHandlers() {
        // Refresh button
        document.getElementById('refresh-btn').addEventListener('click', () => {
            this.refreshData();
        });

        // Deploy form
        document.getElementById('deploy-form').addEventListener('submit', (e) => {
            e.preventDefault();
            this.handleDeploy();
        });

        // Deploy package form
        const deployPkgForm = document.getElementById('deploy-pkg-form');
        if (deployPkgForm) {
            deployPkgForm.addEventListener('submit', async (e) => {
                e.preventDefault();
                const fd = new FormData(deployPkgForm);
                try {
                    this.showLoading('Deploying package...');
                    const res = await fetch('/api/deploy-package', { method: 'POST', headers: { 'Authorization': `Bearer ${this.sessionToken}` }, body: fd });
                    if (res.ok) {
                        const info = await res.json().catch(() => null);
                        this.showSuccess('Package deployed');
                        this.addActivity(`Deployed package${info?.name ? `: ${info.name}` : ''}`);
                        deployPkgForm.reset();
                        document.getElementById('pkg-preview')?.classList.add('hidden');
                        this.switchView('components');
                    } else {
                        const t = await res.text();
                        this.showError(`Package deploy failed: ${t}`);
                    }
                } finally {
                    this.hideLoading();
                }
            });

            // Inspect package preview
            const inspectBtn = document.getElementById('inspect-pkg');
            const fileInput = document.getElementById('deploy-pkg-file');
            if (inspectBtn && fileInput) {
                inspectBtn.addEventListener('click', async () => {
                    const file = fileInput.files && fileInput.files[0];
                    if (!file) { this.showError('Select a package file first'); return; }
                    const fd = new FormData();
                    fd.append('file', file);
                    try {
                        this.showLoading('Inspecting package...');
                        const res = await fetch('/api/deploy-package/inspect', { method: 'POST', headers: { 'Authorization': `Bearer ${this.sessionToken}` }, body: fd });
                        if (!res.ok) {
                            const t = await res.text();
                            this.showError(`Inspect failed: ${t}`);
                            return;
                        }
                        const data = await res.json();
                        this.renderPackagePreview(data);
                    } finally {
                        this.hideLoading();
                    }
                });
            }
        }

        // Discover nodes
        document.getElementById('discover-nodes').addEventListener('click', () => {
            this.discoverNodes();
        });

        // Ops: Apply manifest
        const applyForm = document.getElementById('apply-form');
        if (applyForm) {
            applyForm.addEventListener('submit', async (e) => {
                e.preventDefault();
                const fd = new FormData(applyForm);
                try {
                    this.showLoading('Applying manifest...');
                    const res = await fetch('/api/apply', { method: 'POST', headers: { 'Authorization': `Bearer ${this.sessionToken}` }, body: fd });
                    if (res.ok) { this.showSuccess('Manifest applied'); this.addActivity('Applied manifest'); }
                    else { this.showError('Apply failed'); }
                } finally { this.hideLoading(); }
            });
        }

        // Ops: Upgrade agent
        const upgradeForm = document.getElementById('upgrade-form');
        if (upgradeForm) {
            const addBtn = document.getElementById('add-upgrade-row');
            const rows = document.getElementById('upgrade-rows');
            if (addBtn && rows) {
                addBtn.addEventListener('click', () => {
                    const row = document.createElement('div');
                    row.className = 'grid grid-cols-1 md:grid-cols-2 gap-3';
                    row.innerHTML = `
                        <input type="file" name="file" accept="*/*" required class="w-full bg-graphite border border-graphite rounded px-3 py-2">
                        <input type="text" name="platform" placeholder="platform (e.g., linux/x86_64)" class="w-full bg-graphite border border-graphite rounded px-3 py-2">
                    `;
                    rows.appendChild(row);
                });
            }
            upgradeForm.addEventListener('submit', async (e) => {
                e.preventDefault();
                const fd = new FormData(upgradeForm);
                try {
                    this.showLoading('Upgrading agent...');
                    const res = await fetch('/api/upgrade', { method: 'POST', headers: { 'Authorization': `Bearer ${this.sessionToken}` }, body: fd });
                    if (res.ok) { this.showSuccess('Upgrade published'); this.addActivity('Agent upgrade published'); }
                    else { this.showError('Upgrade failed'); }
                } finally { this.hideLoading(); }
            });
        }

        // Ops: Connect peer
        const connectForm = document.getElementById('connect-form');
        if (connectForm) {
            connectForm.addEventListener('submit', async (e) => {
                e.preventDefault();
                const addr = document.getElementById('peer-addr').value.trim();
                if (!addr) return;
                try {
                    this.showLoading('Connecting to peer...');
                    const res = await this.apiCall('/api/connect', { method: 'POST', body: JSON.stringify({ addr }) });
                    if (res.ok) { this.showSuccess('Peer added to bootstrap'); this.addActivity(`Connect: ${addr}`); }
                } finally { this.hideLoading(); }
            });
        }

        // Policy: buttons
        const savePol = document.getElementById('policy-save');
        const refreshPol = document.getElementById('policy-refresh');
        if (savePol && refreshPol) {
            savePol.addEventListener('click', async () => {
                try {
                    const allow_native_execution = !!document.getElementById('policy-native').checked;
                    const allow_emulation = !!document.getElementById('policy-qemu').checked;
                    const body = { allow_native_execution, allow_emulation };
                    await this.apiCall('/api/policy', { method: 'POST', body: JSON.stringify(body) });
                    this.showSuccess('Policy saved');
                } catch (e) { this.showError('Failed to save policy'); }
            });
            refreshPol.addEventListener('click', async () => {
                await this.refreshPolicyCard();
            });
        }

        // Storage: GC and refresh
        const gcBtn = document.getElementById('storage-gc');
        const refreshStorageBtn = document.getElementById('storage-refresh');
        if (gcBtn && refreshStorageBtn) {
            gcBtn.addEventListener('click', async () => {
                const tgt = parseInt(document.getElementById('storage-gc-target').value || '0', 10);
                if (!Number.isFinite(tgt) || tgt <= 0) { this.showError('Enter target bytes'); return; }
                try {
                    await this.apiCall('/api/storage/gc', { method: 'POST', body: JSON.stringify({ target_total_bytes: tgt }) });
                    this.showSuccess('GC complete');
                    await this.refreshStorageList();
                } catch (_) { this.showError('GC failed'); }
            });
            refreshStorageBtn.addEventListener('click', async () => {
                await this.refreshStorageList();
            });
        }

        // Navigate to Deploy from Components header action
        const newComponentBtn = document.getElementById('new-component');
        if (newComponentBtn) {
            newComponentBtn.addEventListener('click', () => this.switchView('deploy'));
        }

        // Log controls
        document.getElementById('auto-scroll').addEventListener('click', (e) => {
            this.autoScroll = !this.autoScroll;
            e.currentTarget.dataset.active = this.autoScroll;
            e.currentTarget.textContent = this.autoScroll ? 'Auto-scroll' : 'Manual';
            e.currentTarget.classList.toggle('bg-green-600', this.autoScroll);
            e.currentTarget.classList.toggle('text-white', this.autoScroll);
            e.currentTarget.classList.toggle('border', !this.autoScroll);
            e.currentTarget.classList.toggle('border-graphite', !this.autoScroll);
        });

        document.getElementById('clear-logs').addEventListener('click', () => {
            this.clearLogs();
        });

        // P2P auto-scroll toggle
        const p2pAuto = document.getElementById('p2p-auto-scroll');
        if (p2pAuto) {
            p2pAuto.addEventListener('click', (e) => {
                this.autoScroll = !this.autoScroll;
                e.currentTarget.dataset.active = this.autoScroll;
                e.currentTarget.textContent = this.autoScroll ? 'Auto-scroll' : 'Manual';
                e.currentTarget.classList.toggle('bg-green-600', this.autoScroll);
                e.currentTarget.classList.toggle('text-white', this.autoScroll);
                e.currentTarget.classList.toggle('border', !this.autoScroll);
                e.currentTarget.classList.toggle('border-graphite', !this.autoScroll);
            });
        }

        // Log component selector change
        const logSelect = document.getElementById('log-component');
        if (logSelect) {
            logSelect.addEventListener('change', (e) => {
                this.selectedLogComponent = e.currentTarget.value || '__all__';
                this.loadLogsData();
            });
        }

        // Modal controls
        document.querySelectorAll('.modal-close, .modal-cancel').forEach(btn => {
            btn.addEventListener('click', () => this.hideModal());
        });

        const overlay = document.getElementById('modal-overlay');
        if (overlay) {
            overlay.addEventListener('click', (e) => {
                if (e.target === overlay) {
                    this.hideModal();
                }
            });
        }
    }

    switchView(viewName) {
        // Update navigation styles
        if (this.navItems && this.navItems.length) {
            this.navItems.forEach(item => {
                item.classList.remove('bg-neon-blue', 'text-white');
                item.classList.add('text-gray-400');
            });
            const activeItem = this.navItems.find(i => i.dataset.view === viewName);
            if (activeItem) {
                activeItem.classList.add('bg-neon-blue', 'text-white');
                activeItem.classList.remove('text-gray-400');
            }
        }

        // Update view visibility
        document.querySelectorAll('.view').forEach(view => {
            view.classList.add('hidden');
        });
        const activeView = document.getElementById(`${viewName}-view`);
        if (activeView) activeView.classList.remove('hidden');

        this.currentView = viewName;

        // Load view-specific data
        this.loadViewData(viewName);
    }

    async loadViewData(viewName) {
        switch (viewName) {
            case 'overview':
                await this.loadOverviewData();
                break;
            case 'nodes':
                await this.loadNodesData();
                break;
            case 'components':
                await this.loadComponentsData();
                break;
            case 'jobs':
                await loadJobsDataModule(this);
                setupJobsHandlersModule(this);
                break;
            case 'fleet':
                await this.loadFleetHealthData();
                break;
            case 'ops':
                await this.loadVolumes();
                await this.loadDeployHistory();
                await this.refreshPolicyCard();
                await this.refreshStorageList();
                break;
            case 'logs':
                await populateLogComponentsModule(this);
                await loadLogsDataModule(this);
                break;
            case 'p2p':
                // Clear P2P container when entering view
                const p2pContainer = document.getElementById('p2p-container');
                if (p2pContainer) p2pContainer.innerHTML = '';
                break;
        }
    }

    async loadInitialData() {
        await this.loadOverviewData();
    }

    async refreshData() {
        await this.loadViewData(this.currentView);
        this.addActivity('Data refreshed');
    }

    async loadOverviewData() {
        try {
            const response = await apiCall(this.sessionToken, '/api/status');
            const data = await response.json();

            // Update metrics
            document.getElementById('node-count').textContent = data.nodes || 0;
            document.getElementById('component-count').textContent = data.components || 0;
            document.getElementById('system-load').textContent = (data.cpu_avg || 0) + '%';

        } catch (error) {
            console.error('Failed to load overview data:', error);
            this.showError('Failed to load overview data');
        }
    }

    async loadNodesData() {
        try {
            const response = await apiCall(this.sessionToken, '/api/nodes');
            const nodes = await response.json();

            const tbody = document.getElementById('nodes-tbody');
            tbody.innerHTML = '';

            if (nodes.length === 0) {
                tbody.innerHTML = '<tr><td colspan="7" class="loading">No nodes found</td></tr>';
                return;
            }

            nodes.forEach(node => {
                const row = document.createElement('tr');
                const statusHtml = node.online
                    ? '<span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-green-900/30 text-green-400">Online</span>'
                    : '<span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-red-900/30 text-red-400">Offline</span>';
                row.innerHTML = `
                    <td class="p-4" title="${node.id}">${node.alias ? `<span class=\"font-medium\">${node.alias}</span><div class=\"text-xs text-gray-400\">${node.id.substring(0, 12)}...</div>` : `${node.id.substring(0, 12)}...`}</td>
                    <td class="p-4">${statusHtml}</td>
                    <td class="p-4">${node.roles.join(', ')}</td>
                    <td class="p-4">${node.components_running}/${node.components_desired}</td>
                    <td class="p-4">${node.cpu_percent}%</td>
                    <td class="p-4">${node.mem_percent}%</td>
                    <td class="p-4">
                        <button class="border border-graphite px-2 py-1 rounded text-sm hover:bg-graphite" onclick="app.viewNodeDetails('${node.id}')">Details</button>
                    </td>
                `;
                tbody.appendChild(row);
            });

        } catch (error) {
            console.error('Failed to load nodes data:', error);
            document.getElementById('nodes-tbody').innerHTML = '<tr><td colspan="7" class="loading">Failed to load nodes</td></tr>';
        }
    }

    async loadComponentsData() {
        try {
            const response = await apiCall(this.sessionToken, '/api/components');
            const components = await response.json();

            const tbody = document.getElementById('components-tbody');
            tbody.innerHTML = '';

            if (components.length === 0) {
                tbody.innerHTML = '<tr><td colspan="6" class="loading">No components deployed</td></tr>';
                return;
            }

            let total = 0, running = 0, stopped = 0;
            components.forEach(component => {
                total += 1;
                if (component.running) running += 1; else stopped += 1;
                const row = document.createElement('tr');
                const statusHtml = component.running
                    ? '<span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-green-900/30 text-green-400">Running</span>'
                    : '<span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-red-900/30 text-red-400">Stopped</span>';
                row.className = 'border-b border-graphite hover:bg-graphite';
                row.innerHTML = `
                    <td class="p-4">${component.name}</td>
                    <td class="p-4">${statusHtml}</td>
                    <td class="p-4"><span class="bg-azure/20 text-azure px-2 py-1 rounded text-xs">${component.replicas_running}/${component.replicas_desired}</span></td>
                    <td class="p-4">${component.memory_mb}MB</td>
                    <td class="p-4">${component.nodes.join(', ')}</td>
                    <td class="p-4">
                        <div class="flex gap-2">
                            <button class="text-azure hover:text-neon-blue" onclick="app.restartComponent('${component.name}')">Restart</button>
                            <button class="text-red-400 hover:text-red-300" onclick="app.stopComponent('${component.name}')">Stop</button>
                        </div>
                    </td>
                `;
                tbody.appendChild(row);
            });

            // Update summary stats if present
            const totalEl = document.getElementById('comp-total');
            if (totalEl) totalEl.textContent = total;
            const runningEl = document.getElementById('comp-running');
            if (runningEl) runningEl.textContent = running;
            const stoppedEl = document.getElementById('comp-stopped');
            if (stoppedEl) stoppedEl.textContent = stopped;

        } catch (error) {
            console.error('Failed to load components data:', error);
            document.getElementById('components-tbody').innerHTML = '<tr><td colspan="6" class="loading">Failed to load components</td></tr>';
        }
    }

    async loadLogsData() {
        try {
            const comp = encodeURIComponent(this.selectedLogComponent || '__all__');
            const response = await this.apiCall(`/api/logs?tail=100&component=${comp}`);
            const logs = await response.json();

            const container = document.getElementById('logs-container');
            container.innerHTML = '';

            logs.forEach(log => {
                this.addLogLine(log.timestamp, log.component, log.message);
            });

            if (this.autoScroll) {
                container.scrollTop = container.scrollHeight;
            }

        } catch (error) {
            console.error('Failed to load logs:', error);
        }
    }

    async populateLogComponents() {
        try {
            const resp = await this.apiCall('/api/log-components');
            const comps = await resp.json();
            const select = document.getElementById('log-component');
            if (!select) return;

            const previous = this.selectedLogComponent || '__all__';
            // Reset options
            select.innerHTML = '';
            const allOpt = document.createElement('option');
            allOpt.value = '__all__';
            allOpt.textContent = 'All Components';
            select.appendChild(allOpt);

            comps.forEach(name => {
                const opt = document.createElement('option');
                opt.value = name;
                opt.textContent = name;
                select.appendChild(opt);
            });

            // Restore selection if present
            if ([...select.options].some(o => o.value === previous)) {
                select.value = previous;
            } else {
                select.value = '__all__';
            }
            this.selectedLogComponent = select.value;
        } catch (e) {
            // Non-fatal
            console.warn('Failed to populate log components', e);
        }
    }

    async handleDeploy() {
        const form = document.getElementById('deploy-form');
        const formData = new FormData(form);
        try {
            this.showLoading('Deploying component...');
            const response = await fetch('/api/deploy-multipart', {
                method: 'POST',
                headers: {
                    'Authorization': `Bearer ${this.sessionToken}`,
                },
                body: formData
            });
            if (response.ok) {
                this.showSuccess('Component deployed successfully');
                form.reset();
                this.switchView('components');
                this.addActivity(`Deployed component: ${formData.get('name')}`);
            } else {
                const error = await response.text();
                this.showError(`Deployment failed: ${error}`);
            }
        } catch (error) {
            console.error('Deployment error:', error);
            this.showError('Deployment failed: Network error');
        } finally {
            this.hideLoading();
        }
    }

    setupJobsHandlers() {
        const filter = document.getElementById('job-status-filter');
        if (filter && !filter.dataset.bound) {
            filter.addEventListener('change', () => this.loadJobsData());
            filter.dataset.bound = '1';
        }
        const newJobBtn = document.getElementById('new-job');
        if (newJobBtn && !newJobBtn.dataset.bound) {
            newJobBtn.addEventListener('click', () => this.openJobSubmitModal());
            newJobBtn.dataset.bound = '1';
        }
    }

    async loadJobsData() {
        try {
            const filter = document.getElementById('job-status-filter');
            const qs = filter && filter.value ? `?status=${encodeURIComponent(filter.value)}` : '';
            const resp = await this.apiCall(`/api/jobs${qs}`);
            const jobs = await resp.json();

            // Update stats
            const total = jobs.length;
            const counts = { pending: 0, running: 0, completed: 0 };
            jobs.forEach(j => {
                const s = (j.status || '').toLowerCase();
                if (s in counts) counts[s] += 1;
            });
            const el = (id, v) => { const x = document.getElementById(id); if (x) x.textContent = v; };
            el('jobs-total', total);
            el('jobs-pending', counts.pending || 0);
            el('jobs-running', counts.running || 0);
            el('jobs-completed', counts.completed || 0);

            // Table
            const tbody = document.getElementById('jobs-tbody');
            if (!tbody) return;
            tbody.innerHTML = '';
            if (!jobs.length) {
                tbody.innerHTML = '<tr><td colspan="8" class="p-4 text-gray-400">No jobs</td></tr>';
                return;
            }
            jobs.forEach(job => {
                const row = document.createElement('tr');
                row.className = 'border-b border-graphite hover:bg-graphite';
                const status = job.status || '-';
                const type = job.spec?.job_type || '-';
                const node = job.assigned_node || '-';
                const submitted = job.submitted_at ? new Date(job.submitted_at * 1000).toLocaleString() : '-';
                const duration = job.started_at && job.completed_at ? `${Math.max(0, job.completed_at - job.started_at)}s` : '-';
                const idShort = job.id?.slice(0, 12) || '-';
                row.innerHTML = `
                    <td class="p-4" title="${job.id}">${idShort}</td>
                    <td class="p-4">${job.spec?.name || '-'}</td>
                    <td class="p-4">${status}</td>
                    <td class="p-4">${type}</td>
                    <td class="p-4">${node}</td>
                    <td class="p-4">${submitted}</td>
                    <td class="p-4">${duration}</td>
                    <td class="p-4">
                        <button class="text-azure hover:text-neon-blue mr-2" onclick="app.viewJob('${job.id}')">View</button>
                        <button class="text-red-400 hover:text-red-300" onclick="app.cancelJob('${job.id}')">Cancel</button>
                    </td>
                `;
                tbody.appendChild(row);
            });
        } catch (e) {
            console.error('Failed to load jobs', e);
        }
    }

    openJobSubmitModal() { openJobSubmitModalModule(this); }

    async viewJob(jobId) { return viewJobModule(this, jobId); }

    async cancelJob(jobId) { return cancelJobModule(this, jobId); }

    async discoverNodes() {
        try {
            showLoading('Discovering nodes...');
            await apiCall(this.sessionToken, '/api/discover', { method: 'POST' });
            showSuccess('Node discovery started');
            setTimeout(() => this.loadNodesData(), 3000);
        } catch (error) {
            this.showError('Failed to start node discovery');
        } finally {
            hideLoading();
        }
    }

    async restartComponent(name) {
        if (!confirm(`Restart component "${name}"?`)) return;

        try {
            await apiCall(this.sessionToken, `/api/components/${name}/restart`, { method: 'POST' });
            showSuccess(`Component "${name}" restart initiated`);
            addActivity(`Restarted component: ${name}`);
            this.loadComponentsData();
        } catch (error) {
            showError(`Failed to restart component: ${error.message}`);
        }
    }

    async stopComponent(name) {
        if (!confirm(`Stop component "${name}"?`)) return;

        try {
            await apiCall(this.sessionToken, `/api/components/${name}/stop`, { method: 'POST' });
            showSuccess(`Component "${name}" stopped`);
            addActivity(`Stopped component: ${name}`);
            this.loadComponentsData();
        } catch (error) {
            showError(`Failed to stop component: ${error.message}`);
        }
    }

    async refreshPolicyCard() {
        try {
            const polRes = await this.apiCall('/api/policy');
            const pol = await polRes.json();
            const qemuRes = await this.apiCall('/api/qemu/status');
            const qemu = await qemuRes.json();
            const nativeEl = document.getElementById('policy-native');
            const qemuEl = document.getElementById('policy-qemu');
            const qemuStatus = document.getElementById('qemu-status');
            if (nativeEl) nativeEl.checked = !!pol.allow_native_execution;
            if (qemuEl) qemuEl.checked = !!pol.allow_emulation;
            if (qemuStatus) qemuStatus.textContent = `QEMU: ${qemu.qemu_installed ? 'installed' : 'not detected'}`;
        } catch (e) {
            // Non-fatal
        }
    }

    async refreshStorageList() {
        try {
            const res = await this.apiCall('/api/storage');
            const items = await res.json();
            const root = document.getElementById('storage-list');
            if (!root) return;
            if (!items.length) { root.textContent = 'No blobs'; return; }
            root.innerHTML = '';
            items.forEach(it => {
                const row = document.createElement('div');
                row.className = 'flex items-center justify-between border-b border-graphite py-1';
                const short = it.digest.slice(0, 12);
                row.innerHTML = `
                    <div>
                        <div class="font-mono text-xs">${short}…</div>
                        <div class="text-[10px] text-gray-400">${it.size_bytes} bytes • last ${it.last_accessed_unix} • ${it.pinned ? 'pinned' : ''}</div>
                    </div>
                    <div class="flex gap-2">
                        <button class="border border-graphite px-2 py-1 rounded text-[10px]" data-act="pin">${it.pinned ? 'Unpin' : 'Pin'}</button>
                    </div>
                `;
                row.querySelector('[data-act="pin"]').addEventListener('click', async () => {
                    try {
                        await this.apiCall('/api/storage/pin', { method: 'POST', body: JSON.stringify({ digest: it.digest, pinned: !it.pinned }) });
                        await this.refreshStorageList();
                    } catch (_) { this.showError('Pin operation failed'); }
                });
                root.appendChild(row);
            });
        } catch (_) {
            const root = document.getElementById('storage-list');
            if (root) root.textContent = 'Failed to load storage';
        }
    }

    connectWebSocket() {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${protocol}//${window.location.host}/ws?token=${this.sessionToken}`;
        
        this.websocket = new WebSocket(wsUrl);
        
        this.websocket.onopen = () => {
            console.log('WebSocket connected');
            addActivity('Real-time updates connected');
        };
        
        this.websocket.onmessage = (event) => {
            const data = JSON.parse(event.data);
            this.handleWebSocketMessage(data);
        };
        
        this.websocket.onclose = () => {
            console.log('WebSocket disconnected');
            // Reconnect after 5 seconds
            setTimeout(() => this.connectWebSocket(), 5000);
        };
        
        this.websocket.onerror = (error) => {
            console.error('WebSocket error:', error);
        };
    }

    handleWebSocketMessage(data) {
        switch (data.type) {
            case 'metrics':
                this.updateMetrics(data.data);
                break;
            case 'log':
                addLogLine(data.timestamp, data.component, data.message, this.autoScroll);
                // Auto-refresh job list on job events
                if (this.currentView === 'jobs' && data.component === 'system' && (data.message.startsWith('job received') || data.message.startsWith('job started'))) {
                    loadJobsDataModule(this);
                }
                break;
            case 'node_status':
                this.updateNodeStatus(data.node_id, data.status);
                break;
            case 'component_status':
                this.updateComponentStatus(data.component, data.status);
                break;
            case 'activity':
                addActivity(data.message);
                break;
            case 'p2p_event':
                this.addP2PLine(data.timestamp, data.direction, data.source, data.topic, data.message);
                break;
        }
    }

    updateMetrics(metrics) {
        // Update overview cards with real-time data
        document.getElementById('node-count').textContent = metrics.nodes || 1;
        document.getElementById('component-count').textContent = metrics.components_running || 0;
        document.getElementById('system-load').textContent = `${Math.round((metrics.mem_current_bytes || 0) / (1024 * 1024))}MB`;
        
        // Add activity for significant changes
        if (metrics.components_running !== this.lastMetrics?.components_running) {
            this.addActivity(`Components running: ${metrics.components_running || 0}`);
        }
        
        this.lastMetrics = metrics;
    }

    renderPackagePreview(data) {
        const box = document.getElementById('pkg-preview');
        const comp = document.getElementById('pkg-component');
        const mounts = document.getElementById('pkg-mounts');
        const files = document.getElementById('pkg-files');
        if (!box || !comp || !mounts || !files) return;
        comp.textContent = `${data.component?.name || '-'} → ${data.component?.wasm || '-'} (${data.component?.sha256 || 'no sha'})`;
        mounts.innerHTML = '';
        (data.mounts || []).forEach(m => {
            const li = document.createElement('li');
            li.textContent = `${m.kind} ${m.ro ? '(ro)' : '(rw)'}: ${m.host} → ${m.guest}`;
            mounts.appendChild(li);
        });
        files.innerHTML = '';
        (data.files || []).forEach(f => {
            const d = document.createElement('div');
            d.textContent = `${f.is_dir ? '📁' : '📄'} ${f.path}${!f.is_dir ? ` (${f.size} bytes)` : ''}`;
            files.appendChild(d);
        });
        box.classList.remove('hidden');
    }

    async loadVolumes() {
        try {
            const res = await this.apiCall('/api/volumes');
            const vols = await res.json();
            const root = document.getElementById('volumes-list');
            if (!root) return;
            if (!vols.length) { root.textContent = 'No persistent volumes'; return; }
            root.innerHTML = '';
            vols.forEach(v => {
                const row = document.createElement('div');
                row.className = 'flex items-center justify-between border-b border-graphite py-2';
                row.innerHTML = `
                    <div>
                        <div class="font-mono text-xs text-gray-300">${v.path}</div>
                        <div class="text-xs text-gray-400">${v.name} • ${v.size_mb} MB • ${v.files} files</div>
                    </div>
                    <div class="flex gap-2">
                        <button class="border border-graphite px-2 py-1 rounded text-xs" data-act="open" data-path="${v.path}">Open</button>
                        <button class="text-red-400 hover:text-red-300 text-xs" data-act="clear" data-name="${v.name}">Clear</button>
                    </div>
                `;
                // Bind actions
                row.querySelector('[data-act="clear"]').addEventListener('click', async (e) => {
                    const name = e.currentTarget.getAttribute('data-name');
                    if (!confirm(`Clear volume \"${name}\"? This cannot be undone.`)) return;
                    try {
                        await this.apiCall('/api/volumes/clear', { method: 'POST', body: JSON.stringify({ name }) });
                        this.showSuccess('Volume cleared');
                        this.loadVolumes();
                    } catch (err) {
                        this.showError('Failed to clear volume');
                    }
                });
                row.querySelector('[data-act="open"]').addEventListener('click', (e) => {
                    const p = e.currentTarget.getAttribute('data-path');
                    this.showModal('Volume Path', `<pre class="text-xs whitespace-pre-wrap">${p}</pre>`);
                });
                root.appendChild(row);
            });
        } catch (e) {
            const root = document.getElementById('volumes-list');
            if (root) root.textContent = 'Failed to load volumes';
        }
    }

    async loadDeployHistory() {
        try {
            const res = await this.apiCall('/api/deploy-history');
            const items = await res.json();
            const box = document.getElementById('deploy-history');
            if (!box) return;
            if (!items.length) { box.textContent = 'No deployments yet.'; return; }
            box.innerHTML = '';
            items.forEach(it => {
                const div = document.createElement('div');
                div.className = 'border-b border-graphite py-2';
                const ts = it.timestamp ? new Date(it.timestamp * 1000).toLocaleString() : 'now';
                div.innerHTML = `
                    <div class="text-sm">${it.component} — <span class="font-mono">${it.digest?.slice(0, 12) || '-'}</span></div>
                    <div class="text-xs text-gray-400">${ts}</div>
                `;
                box.appendChild(div);
            });
        } catch (e) {
            const box = document.getElementById('deploy-history');
            if (box) box.textContent = 'Failed to load deployment history';
        }
    }

    async loadFleetHealthData() {
        try {
            // Load fleet health overview
            const fleetRes = await this.apiCall('/api/health/fleet');
            const fleetHealth = await fleetRes.json();
            
            // Update fleet overview cards
            this.updateFleetOverview(fleetHealth);
            
            // Load node health
            const nodesRes = await this.apiCall('/api/health/nodes');
            const nodeHealth = await nodesRes.json();
            this.updateNodeHealthTable(nodeHealth);
            
            // Load component health
            const componentsRes = await this.apiCall('/api/health/components');
            const componentHealth = await componentsRes.json();
            this.updateComponentHealthTable(componentHealth);
            
        } catch (e) {
            console.error('Failed to load fleet health data:', e);
            this.showError('Failed to load fleet health data');
        }
    }

    updateFleetOverview(fleetHealth) {
        // Update status
        const statusEl = document.getElementById('fleet-status');
        const statusIconEl = document.getElementById('fleet-status-icon');
        if (statusEl && statusIconEl) {
            let statusText, statusClass, iconClass;
            switch (fleetHealth.overall_status) {
                case 'healthy':
                    statusText = 'Healthy';
                    statusClass = 'text-green-400';
                    iconClass = 'text-green-400';
                    break;
                case 'warning':
                    statusText = 'Warning';
                    statusClass = 'text-yellow-400';
                    iconClass = 'text-yellow-400';
                    break;
                case 'critical':
                    statusText = 'Critical';
                    statusClass = 'text-red-400';
                    iconClass = 'text-red-400';
                    break;
                default:
                    statusText = 'Unknown';
                    statusClass = 'text-gray-400';
                    iconClass = 'text-gray-400';
            }
            statusEl.textContent = statusText;
            statusEl.className = `text-lg font-semibold ${statusClass}`;
            statusIconEl.className = `text-2xl ${iconClass}`;
        }

        // Update metrics
        document.getElementById('fleet-nodes').textContent = fleetHealth.total_nodes;
        document.getElementById('fleet-components').textContent = `${fleetHealth.healthy_components}/${fleetHealth.total_components}`;
        document.getElementById('fleet-response').textContent = `${Math.round(fleetHealth.average_response_time)}ms`;

        // Update system metrics
        document.getElementById('system-memory').textContent = `${Math.round(fleetHealth.memory_usage_percent)}%`;
        document.getElementById('system-memory-bar').style.width = `${fleetHealth.memory_usage_percent}%`;
        document.getElementById('system-disk').textContent = `${Math.round(fleetHealth.disk_usage_percent)}%`;
        document.getElementById('system-disk-bar').style.width = `${fleetHealth.disk_usage_percent}%`;
        
        // Format uptime
        const uptimeHours = Math.floor(fleetHealth.uptime_seconds / 3600);
        const uptimeText = uptimeHours > 0 ? `${uptimeHours}h` : `${Math.floor(fleetHealth.uptime_seconds / 60)}m`;
        document.getElementById('system-uptime').textContent = uptimeText;

        // Update alerts
        this.updateAlertsList(fleetHealth.checks);
    }

    updateAlertsList(checks) {
        const alertsList = document.getElementById('alerts-list');
        const alertCount = document.getElementById('alert-count');
        if (!alertsList || !alertCount) return;

        // Find issues from health checks
        const alerts = checks.filter(check => check.status !== 'healthy');
        
        alertCount.textContent = alerts.length;
        alertCount.className = alerts.length > 0 ? 'bg-red-600 text-white px-2 py-1 rounded-full text-xs' : 'bg-gray-600 text-white px-2 py-1 rounded-full text-xs';
        
        if (alerts.length === 0) {
            alertsList.innerHTML = '<p class="text-gray-400 text-sm">No active alerts</p>';
            return;
        }

        alertsList.innerHTML = '';
        alerts.forEach(alert => {
            const div = document.createElement('div');
            div.className = `border border-graphite rounded p-3 ${alert.status === 'critical' ? 'border-red-500 bg-red-900/20' : 'border-yellow-500 bg-yellow-900/20'}`;
            div.innerHTML = `
                <div class="flex items-start justify-between">
                    <div>
                        <div class="font-medium ${alert.status === 'critical' ? 'text-red-400' : 'text-yellow-400'}">${alert.component}</div>
                        <div class="text-sm text-gray-300 mt-1">${alert.message}</div>
                        <div class="text-xs text-gray-400 mt-2">${new Date(alert.last_check * 1000).toLocaleString()}</div>
                    </div>
                    <span class="text-xs px-2 py-1 rounded ${alert.status === 'critical' ? 'bg-red-600 text-white' : 'bg-yellow-600 text-white'}">${alert.status}</span>
                </div>
            `;
            alertsList.appendChild(div);
        });
    }

    updateNodeHealthTable(nodeHealth) {
        const tbody = document.getElementById('node-health-table');
        if (!tbody) return;

        if (nodeHealth.length === 0) {
            tbody.innerHTML = '<tr><td colspan="8" class="p-4 text-gray-400">No nodes found</td></tr>';
            return;
        }

        tbody.innerHTML = '';
        nodeHealth.forEach(node => {
            const row = document.createElement('tr');
            row.className = 'border-b border-graphite/50 hover:bg-graphite/30';
            
            const statusClass = node.status === 'healthy' ? 'text-green-400' : 
                               node.status === 'warning' ? 'text-yellow-400' : 'text-red-400';
            
            const alertsText = node.alerts.length > 0 ? `${node.alerts.length} alert${node.alerts.length > 1 ? 's' : ''}` : 'None';
            
            row.innerHTML = `
                <td class="p-3 font-mono text-sm">${node.node_id.length > 16 ? node.node_id.substring(0, 16) + '...' : node.node_id}</td>
                <td class="p-3"><span class="${statusClass} capitalize">${node.status}</span></td>
                <td class="p-3">${node.components_running}/${node.components_desired}</td>
                <td class="p-3">${node.cpu_percent}%</td>
                <td class="p-3">${node.memory_percent}%</td>
                <td class="p-3 text-sm">${node.platform}</td>
                <td class="p-3">v${node.agent_version}</td>
                <td class="p-3 text-sm ${node.alerts.length > 0 ? 'text-yellow-400' : 'text-gray-400'}">${alertsText}</td>
            `;
            tbody.appendChild(row);
        });
    }

    updateComponentHealthTable(componentHealth) {
        const tbody = document.getElementById('component-health-table');
        if (!tbody) return;

        if (componentHealth.length === 0) {
            tbody.innerHTML = '<tr><td colspan="7" class="p-4 text-gray-400">No components found</td></tr>';
            return;
        }

        tbody.innerHTML = '';
        componentHealth.forEach(component => {
            const row = document.createElement('tr');
            row.className = 'border-b border-graphite/50 hover:bg-graphite/30';
            
            const statusClass = component.status === 'healthy' ? 'text-green-400' : 
                               component.status === 'warning' ? 'text-yellow-400' : 'text-red-400';
            
            const lastRestart = component.last_restart ? 
                new Date(component.last_restart * 1000).toLocaleString() : 'Never';
            
            row.innerHTML = `
                <td class="p-3 font-medium">${component.name}</td>
                <td class="p-3"><span class="${statusClass} capitalize">${component.status}</span></td>
                <td class="p-3">${component.replicas_running}/${component.replicas_desired}</td>
                <td class="p-3">${component.restart_count}</td>
                <td class="p-3">${component.error_rate.toFixed(1)}%</td>
                <td class="p-3">${component.memory_usage_mb}MB</td>
                <td class="p-3 text-sm text-gray-400">${lastRestart}</td>
            `;
            tbody.appendChild(row);
        });
    }

    addLogLine(timestamp, component, message) {
        const container = document.getElementById('logs-container');
        const line = document.createElement('div');
        line.className = 'flex gap-4 mb-1 whitespace-nowrap';
        line.innerHTML = `
            <span class="text-gray-400 min-w-[150px]">${timestamp}</span>
            <span class="text-yellow-400 min-w-[100px]">${component}</span>
            <span class="text-gray-100 whitespace-pre-wrap">${message}</span>
        `;
        container.appendChild(line);

        // Keep only last 1000 lines
        while (container.children.length > 1000) {
            container.removeChild(container.firstChild);
        }

        if (this.autoScroll) {
            container.scrollTop = container.scrollHeight;
        }
    }
   
    // Render a P2P message in the P2P view
    addP2PLine(timestamp, direction, source, topic, message) {
        const container = document.getElementById('p2p-container');
        if (!container) return;
        const line = document.createElement('div');
        line.className = 'flex gap-2 mb-1 whitespace-pre-wrap';
        line.innerHTML = `
            <span class="text-gray-400 min-w-[80px]">${timestamp}</span>
            <span class="text-blue-400 min-w-[60px]">${direction}</span>
            <span class="text-yellow-400 min-w-[100px]">${source}</span>
            <span class="text-green-400 min-w-[100px]">${topic}</span>
            <span>${message}</span>
        `;
        container.appendChild(line);
        // Keep only last 1000 events
        while (container.children.length > 1000) {
            container.removeChild(container.firstChild);
        }
        if (this.autoScroll) {
            container.scrollTop = container.scrollHeight;
        }
    }

    clearLogs() { clearLogs(); }

    addActivity(message) {
        const container = document.getElementById('recent-activity');
        const item = document.createElement('div');
        item.className = 'activity-item';
        item.innerHTML = `
            <span class="activity-time">Just now</span>
            <span class="activity-desc">${message}</span>
        `;
        container.insertBefore(item, container.firstChild);

        // Keep only last 10 activities
        while (container.children.length > 10) {
            container.removeChild(container.lastChild);
        }
    }

    async apiCall(url, options = {}) {
        const defaultOptions = {
            headers: {
                'Authorization': `Bearer ${this.sessionToken}`,
                'Content-Type': 'application/json',
                ...options.headers
            }
        };

        const response = await fetch(url, { ...defaultOptions, ...options });
        
        if (!response.ok) {
            throw new Error(`API call failed: ${response.statusText}`);
        }
        
        return response;
    }

    showModal(title, body, onConfirm = null) { showModal(title, body, onConfirm); }
    hideModal() { hideModal(); }
    showSuccess(message) { showSuccess(message); }
    showError(message) { showError(message); }
    showLoading(message) { showLoading(message); }
    hideLoading() { hideLoading(); }
    showNotification(message, type) { showNotification(message, type); }

    async viewNodeDetails(nodeId) {
        try {
            const res = await this.apiCall(`/api/nodes/${encodeURIComponent(nodeId)}`);
            const details = await res.json();
            const alias = details.alias || '';
            const notes = details.notes || '';
            const roles = (details.roles || []).join(', ');
            const body = `
                <div class="space-y-3 text-sm">
                    <div>
                        <div class="text-gray-400">Node ID</div>
                        <div class="font-mono break-all">${details.id}</div>
                    </div>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                        <div>
                            <label class="block text-gray-300 mb-1">Alias</label>
                            <input id="node-alias" type="text" value="${alias.replace(/"/g, '&quot;')}" class="w-full bg-graphite border border-graphite rounded px-3 py-2">
                        </div>
                        <div>
                            <label class="block text-gray-300 mb-1">Roles</label>
                            <div class="bg-graphite border border-graphite rounded px-3 py-2">${roles || '-'}</div>
                        </div>
                    </div>
                    <div class="grid grid-cols-2 gap-3">
                        <div>
                            <div class="text-gray-400">Components</div>
                            <div>${details.components_running}/${details.components_desired}</div>
                        </div>
                        <div>
                            <div class="text-gray-400">CPU / Mem</div>
                            <div>${details.cpu_percent}% / ${details.mem_percent}%</div>
                        </div>
                    </div>
                    <div>
                        <label class="block text-gray-300 mb-1">Notes</label>
                        <textarea id="node-notes" rows="4" class="w-full bg-graphite border border-graphite rounded px-3 py-2">${notes.replace(/</g,'&lt;')}</textarea>
                    </div>
                </div>`;
            this.showModal('Node Details', body, async () => {
                const newAlias = document.getElementById('node-alias').value.trim();
                const newNotes = document.getElementById('node-notes').value;
                try {
                    await this.apiCall(`/api/nodes/${encodeURIComponent(nodeId)}`, { method: 'POST', body: JSON.stringify({ alias: newAlias, notes: newNotes }) });
                    this.showSuccess('Node updated');
                    if (this.currentView === 'nodes') { this.loadNodesData(); }
                } catch (e) {
                    this.showError('Failed to update node');
                }
            });
        } catch (e) {
            this.showError('Failed to load node details');
        }
    }
}

// Initialize the app when the DOM is loaded
document.addEventListener('DOMContentLoaded', () => {
    window.app = new RealmApp();
});

// Handle page unload
window.addEventListener('beforeunload', () => {
    if (window.app && window.app.websocket) {
        window.app.websocket.close();
    }
});

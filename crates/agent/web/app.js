// Realm Management Web Interface
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
                await this.loadJobsData();
                this.setupJobsHandlers();
                break;
            case 'ops':
                break;
            case 'logs':
                await this.populateLogComponents();
                await this.loadLogsData();
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
            const response = await this.apiCall('/api/status');
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
            const response = await this.apiCall('/api/nodes');
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
                    <td class="p-4" title="${node.id}">${node.id.substring(0, 12)}...</td>
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
            const response = await this.apiCall('/api/components');
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

    openJobSubmitModal() {
        const body = `
            <form id="job-submit-form" class="space-y-3">
                <div>
                    <label class="block text-sm text-gray-300 mb-1">Job TOML</label>
                    <textarea id="job-toml" rows="10" class="w-full bg-graphite border border-graphite rounded px-3 py-2" placeholder="[job]\nname='example'\n type='one-shot'\n\n[runtime]\n type='wasm'\n source='file:///path/to.wasm'\n memory_mb=64\n fuel=5000000\n epoch_ms=100\n"></textarea>
                </div>
                <div class="flex items-center justify-end gap-2">
                    <button type="button" class="border border-graphite px-4 py-2 rounded" id="job-cancel">Close</button>
                    <button type="submit" class="bg-neon-blue hover:bg-azure px-4 py-2 rounded">Submit</button>
                </div>
            </form>
        `;
        this.showModal('Submit Job', body, null);
        const form = document.getElementById('job-submit-form');
        const closeBtn = document.getElementById('job-cancel');
        if (closeBtn) closeBtn.addEventListener('click', () => this.hideModal());
        if (form) {
            form.addEventListener('submit', async (e) => {
                e.preventDefault();
                const toml = document.getElementById('job-toml').value;
                const fd = new FormData();
                fd.append('job_toml', toml);
                try {
                    this.showLoading('Submitting job...');
                    const res = await fetch('/api/jobs/submit', { method: 'POST', headers: { 'Authorization': `Bearer ${this.sessionToken}` }, body: fd });
                    if (res.ok) { this.showSuccess('Job submitted'); this.hideModal(); this.loadJobsData(); }
                    else { this.showError('Submit failed'); }
                } finally { this.hideLoading(); }
            });
        }
    }

    async viewJob(jobId) {
        try {
            const res = await this.apiCall(`/api/jobs/${encodeURIComponent(jobId)}`);
            const job = await res.json();
            const logsHtml = (job.logs || []).map(l => `<div class='text-xs text-gray-300'>${new Date(l.timestamp*1000).toLocaleTimeString()} [${l.level}] ${l.message}</div>`).join('');
            this.showModal('Job Details', `
                <div class='space-y-2'>
                    <div><strong>ID:</strong> ${job.id}</div>
                    <div><strong>Name:</strong> ${job.spec?.name || '-'}</div>
                    <div><strong>Status:</strong> ${job.status}</div>
                    <div><strong>Node:</strong> ${job.assigned_node || '-'}</div>
                    <div><strong>Logs:</strong><div class='mt-2 max-h-64 overflow-y-auto bg-graphite p-2 rounded'>${logsHtml || 'No logs'}</div></div>
                </div>
            `);
        } catch (e) {
            this.showError('Failed to load job');
        }
    }

    async cancelJob(jobId) {
        if (!confirm('Cancel this job?')) return;
        try {
            await this.apiCall(`/api/jobs/${encodeURIComponent(jobId)}/cancel`, { method: 'POST' });
            this.showSuccess('Job cancel requested');
            this.loadJobsData();
        } catch (e) {
            this.showError('Cancel failed');
        }
    }

    async discoverNodes() {
        try {
            this.showLoading('Discovering nodes...');
            await this.apiCall('/api/discover', { method: 'POST' });
            this.showSuccess('Node discovery started');
            setTimeout(() => this.loadNodesData(), 3000);
        } catch (error) {
            this.showError('Failed to start node discovery');
        } finally {
            this.hideLoading();
        }
    }

    async restartComponent(name) {
        if (!confirm(`Restart component "${name}"?`)) return;

        try {
            await this.apiCall(`/api/components/${name}/restart`, { method: 'POST' });
            this.showSuccess(`Component "${name}" restart initiated`);
            this.addActivity(`Restarted component: ${name}`);
            this.loadComponentsData();
        } catch (error) {
            this.showError(`Failed to restart component: ${error.message}`);
        }
    }

    async stopComponent(name) {
        if (!confirm(`Stop component "${name}"?`)) return;

        try {
            await this.apiCall(`/api/components/${name}/stop`, { method: 'POST' });
            this.showSuccess(`Component "${name}" stopped`);
            this.addActivity(`Stopped component: ${name}`);
            this.loadComponentsData();
        } catch (error) {
            this.showError(`Failed to stop component: ${error.message}`);
        }
    }

    connectWebSocket() {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${protocol}//${window.location.host}/ws?token=${this.sessionToken}`;
        
        this.websocket = new WebSocket(wsUrl);
        
        this.websocket.onopen = () => {
            console.log('WebSocket connected');
            this.addActivity('Real-time updates connected');
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
                this.addLogLine(data.timestamp, data.component, data.message);
                break;
            case 'node_status':
                this.updateNodeStatus(data.node_id, data.status);
                break;
            case 'component_status':
                this.updateComponentStatus(data.component, data.status);
                break;
            case 'activity':
                this.addActivity(data.message);
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

    clearLogs() {
        document.getElementById('logs-container').innerHTML = '';
    }

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

    showModal(title, body, onConfirm = null) {
        document.getElementById('modal-title').textContent = title;
        document.getElementById('modal-body').innerHTML = body;
        const overlay = document.getElementById('modal-overlay');
        overlay.classList.remove('hidden');
        overlay.style.display = 'flex';
        
        const confirmBtn = document.querySelector('.modal-confirm');
        if (onConfirm) {
            confirmBtn.style.display = 'block';
            confirmBtn.onclick = () => {
                onConfirm();
                this.hideModal();
            };
        } else {
            confirmBtn.style.display = 'none';
        }
    }

    hideModal() {
        const overlay = document.getElementById('modal-overlay');
        overlay.style.display = 'none';
        overlay.classList.add('hidden');
    }

    showSuccess(message) {
        this.showNotification(message, 'success');
    }

    showError(message) {
        this.showNotification(message, 'error');
    }

    showLoading(message) {
        // You could implement a loading spinner here
        console.log('Loading:', message);
    }

    hideLoading() {
        // Hide loading spinner
    }

    showNotification(message, type) {
        // Simple notification - you could enhance this with a proper toast system
        const notification = document.createElement('div');
        notification.style.cssText = `
            position: fixed;
            top: 20px;
            right: 20px;
            padding: 1rem;
            border-radius: 0.5rem;
            color: white;
            font-weight: 500;
            z-index: 1001;
            background: ${type === 'success' ? '#059669' : '#ef4444'};
        `;
        notification.textContent = message;
        document.body.appendChild(notification);

        setTimeout(() => {
            document.body.removeChild(notification);
        }, 5000);
    }

    viewNodeDetails(nodeId) {
        this.showModal('Node Details', `
            <p><strong>Node ID:</strong> ${nodeId}</p>
            <p>Detailed node information would be displayed here.</p>
        `);
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

// Realm Management Web Interface
class RealmApp {
    constructor() {
        this.currentView = 'overview';
        this.sessionToken = this.getSessionToken();
        this.websocket = null;
        this.autoScroll = true;
        
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
            case 'ops':
                break;
            case 'logs':
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
            const response = await this.apiCall('/api/logs?tail=100');
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

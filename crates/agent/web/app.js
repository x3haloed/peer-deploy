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
        const navItems = document.querySelectorAll('.nav-item');
        navItems.forEach(item => {
            item.addEventListener('click', (e) => {
                const view = e.target.dataset.view;
                this.switchView(view);
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

        // Log controls
        document.getElementById('auto-scroll').addEventListener('click', (e) => {
            this.autoScroll = !this.autoScroll;
            e.target.dataset.active = this.autoScroll;
            e.target.textContent = this.autoScroll ? 'Auto-scroll' : 'Manual';
        });

        document.getElementById('clear-logs').addEventListener('click', () => {
            this.clearLogs();
        });

        // Modal controls
        document.querySelectorAll('.modal-close, .modal-cancel').forEach(btn => {
            btn.addEventListener('click', () => this.hideModal());
        });

        document.querySelector('.modal-overlay').addEventListener('click', (e) => {
            if (e.target.classList.contains('modal-overlay')) {
                this.hideModal();
            }
        });
    }

    switchView(viewName) {
        // Update navigation
        document.querySelectorAll('.nav-item').forEach(item => {
            item.classList.remove('active');
        });
        document.querySelector(`[data-view="${viewName}"]`).classList.add('active');

        // Update view
        document.querySelectorAll('.view').forEach(view => {
            view.classList.remove('active');
        });
        document.getElementById(`${viewName}-view`).classList.add('active');

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
                row.innerHTML = `
                    <td title="${node.id}">${node.id.substring(0, 12)}...</td>
                    <td><span class="status ${node.online ? 'status-online' : 'status-offline'}">${node.online ? 'Online' : 'Offline'}</span></td>
                    <td>${node.roles.join(', ')}</td>
                    <td>${node.components_running}/${node.components_desired}</td>
                    <td>${node.cpu_percent}%</td>
                    <td>${node.mem_percent}%</td>
                    <td>
                        <button class="btn btn-secondary" onclick="app.viewNodeDetails('${node.id}')">Details</button>
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

            components.forEach(component => {
                const row = document.createElement('tr');
                row.innerHTML = `
                    <td>${component.name}</td>
                    <td><span class="status ${component.running ? 'status-running' : 'status-stopped'}">${component.running ? 'Running' : 'Stopped'}</span></td>
                    <td>${component.replicas_running}/${component.replicas_desired}</td>
                    <td>${component.memory_mb}MB</td>
                    <td>${component.nodes.join(', ')}</td>
                    <td>
                        <button class="btn btn-secondary" onclick="app.restartComponent('${component.name}')">Restart</button>
                        <button class="btn btn-danger" onclick="app.stopComponent('${component.name}')">Stop</button>
                    </td>
                `;
                tbody.appendChild(row);
            });

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
        line.className = 'log-line';
        line.innerHTML = `
            <span class="log-time">${timestamp}</span>
            <span class="log-component">${component}</span>
            <span class="log-message">${message}</span>
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
        document.getElementById('modal-overlay').style.display = 'flex';
        
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
        document.getElementById('modal-overlay').style.display = 'none';
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

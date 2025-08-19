import { apiCall } from './api.js';
import { addLogLine } from './utils.js';

export async function populateLogComponents(app) {
	try {
		const resp = await apiCall(app.sessionToken, '/api/log-components');
		const comps = await resp.json();
		const select = document.getElementById('log-component');
		if (!select) return;
		const previous = app.selectedLogComponent || '__all__';
		select.innerHTML = '';
		const allOpt = document.createElement('option');
		allOpt.value = '__all__';
		allOpt.textContent = 'All Components';
		select.appendChild(allOpt);
		comps.forEach(name => {
			const opt = document.createElement('option');
			opt.value = name; opt.textContent = name; select.appendChild(opt);
		});
		if ([...select.options].some(o => o.value === previous)) { select.value = previous; }
		else { select.value = '__all__'; }
		app.selectedLogComponent = select.value;
	} catch (e) { console.warn('Failed to populate log components', e); }
}

export async function loadLogsData(app) {
	try {
		const comp = encodeURIComponent(app.selectedLogComponent || '__all__');
		const response = await apiCall(app.sessionToken, `/api/logs?tail=100&component=${comp}`);
		const logs = await response.json();
		const container = document.getElementById('logs-container');
		container.innerHTML = '';
		logs.forEach(log => { addLogLine(log.timestamp, log.component, log.message, app.autoScroll); });
		if (app.autoScroll) { container.scrollTop = container.scrollHeight; }
	} catch (e) { console.error('Failed to load logs:', e); }
}


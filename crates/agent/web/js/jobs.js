import { apiCall } from './api.js';
import { showModal, hideModal, showLoading, hideLoading, showSuccess, showError } from './utils.js';

export function setupJobsHandlers(app) {
	const filter = document.getElementById('job-status-filter');
	if (filter && !filter.dataset.bound) {
		filter.addEventListener('change', () => loadJobsData(app));
		filter.dataset.bound = '1';
	}
	const newJobBtn = document.getElementById('new-job');
	if (newJobBtn && !newJobBtn.dataset.bound) {
		newJobBtn.addEventListener('click', () => openJobSubmitModal(app));
		newJobBtn.dataset.bound = '1';
	}
}

export async function loadJobsData(app) {
	try {
		const filter = document.getElementById('job-status-filter');
		const qs = filter && filter.value ? `?status=${encodeURIComponent(filter.value)}` : '';
		const resp = await apiCall(app.sessionToken, `/api/jobs${qs}`);
		const jobs = await resp.json();
		const total = jobs.length;
		const counts = { pending: 0, running: 0, completed: 0 };
		jobs.forEach(j => { const s = (j.status || '').toLowerCase(); if (s in counts) counts[s] += 1; });
		const el = (id, v) => { const x = document.getElementById(id); if (x) x.textContent = v; };
		el('jobs-total', total); el('jobs-pending', counts.pending || 0); el('jobs-running', counts.running || 0); el('jobs-completed', counts.completed || 0);
		const tbody = document.getElementById('jobs-tbody'); if (!tbody) return; tbody.innerHTML = '';
		if (!jobs.length) { tbody.innerHTML = '<tr><td colspan="8" class="p-4 text-gray-400">No jobs</td></tr>'; return; }
		jobs.forEach(job => {
			const row = document.createElement('tr'); row.className = 'border-b border-graphite hover:bg-graphite';
			const status = job.status || '-'; const type = job.spec?.job_type || '-'; const node = job.assigned_node || '-';
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
				</td>`;
			tbody.appendChild(row);
		});
	} catch (e) { console.error('Failed to load jobs', e); }
}

export function openJobSubmitModal(app) {
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
		</form>`;
	showModal('Submit Job', body, null);
	const form = document.getElementById('job-submit-form');
	const closeBtn = document.getElementById('job-cancel');
	if (closeBtn) closeBtn.addEventListener('click', () => hideModal());
	if (form) {
		form.addEventListener('submit', async (e) => {
			e.preventDefault();
			const toml = document.getElementById('job-toml').value;
			const fd = new FormData(); fd.append('job_toml', toml);
			try {
				showLoading('Submitting job...');
				const res = await fetch('/api/jobs/submit', { method: 'POST', headers: { 'Authorization': `Bearer ${app.sessionToken}` }, body: fd });
				if (res.ok) { showSuccess('Job submitted'); hideModal(); loadJobsData(app); }
				else { showError('Submit failed'); }
			} finally { hideLoading(); }
		});
	}
}

export async function viewJob(app, jobId) {
	try {
		const res = await apiCall(app.sessionToken, `/api/jobs/${encodeURIComponent(jobId)}`);
		const job = await res.json();
		const logsHtml = (job.logs || []).map(l => `<div class='text-xs text-gray-300'>${new Date(l.timestamp*1000).toLocaleTimeString()} [${l.level}] ${l.message}</div>`).join('');
		showModal('Job Details', `
			<div class='space-y-2'>
				<div><strong>ID:</strong> ${job.id}</div>
				<div><strong>Name:</strong> ${job.spec?.name || '-'}</div>
				<div><strong>Status:</strong> ${job.status}</div>
				<div><strong>Node:</strong> ${job.assigned_node || '-'}</div>
				<div><strong>Logs:</strong><div class='mt-2 max-h-64 overflow-y-auto bg-graphite p-2 rounded'>${logsHtml || 'No logs'}</div></div>
			</div>`);
	} catch (e) { showError('Failed to load job'); }
}

export async function cancelJob(app, jobId) {
	if (!confirm('Cancel this job?')) return;
	try {
		await apiCall(app.sessionToken, `/api/jobs/${encodeURIComponent(jobId)}/cancel`, { method: 'POST' });
		showSuccess('Job cancel requested');
		loadJobsData(app);
	} catch (e) { showError('Cancel failed'); }
}


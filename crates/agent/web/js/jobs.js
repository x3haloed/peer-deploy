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
	const examples = {
		'one-shot': `name = "example-oneshot"
job_type = "one-shot"

[runtime]
type = "wasm"
source = "file:///path/to/task.wasm"
memory_mb = 64
fuel = 5000000
epoch_ms = 100

[execution]
timeout_minutes = 30
artifacts = [
  { path = "/output/result.txt", name = "result.txt" }
]

[targeting]
platform = "linux/x86_64"
tags = ["worker"]`,
		'recurring': `name = "scheduled-backup"
job_type = "recurring"
schedule = "0 2 * * *"  # Daily at 2 AM

[runtime]
type = "wasm"
source = "file:///path/to/backup.wasm"
memory_mb = 128
fuel = 10000000
epoch_ms = 100

[execution]
timeout_minutes = 60
artifacts = [
  { path = "/backup/data.tar.gz", name = "daily-backup.tar.gz" }
]`,
		'service': `name = "web-service"
job_type = "service"

[runtime]
type = "wasm" 
source = "file:///path/to/service.wasm"
memory_mb = 256
fuel = 50000000
epoch_ms = 1000

[targeting]
tags = ["server"]`,
		'native': `name = "native-job"
job_type = "one-shot"

[runtime]
type = "native"
binary = "file:///usr/bin/echo"
args = ["hello", "world"]

[execution]
timeout_minutes = 5

[targeting]
tags = ["builder"]
`,
		'qemu': `name = "qemu-job"
job_type = "one-shot"

[runtime]
type = "qemu"
binary = "file:///path/to/foreign-arch-binary"
args = ["--help"]
target_platform = "linux/amd64"  # or linux/arm64, etc

[execution]
timeout_minutes = 10

[targeting]
tags = ["builder"]
`
	};

	const body = `
		<form id="job-submit-form" class="space-y-4">
			<div>
				<label class="block text-sm text-gray-300 mb-2">Job Type Templates</label>
				<div class="flex space-x-2 mb-3">
					<button type="button" class="text-xs bg-azure hover:bg-neon-blue px-3 py-1 rounded" onclick="setJobTemplate('one-shot')">One-Shot</button>
					<button type="button" class="text-xs bg-azure hover:bg-neon-blue px-3 py-1 rounded" onclick="setJobTemplate('recurring')">Recurring</button>
					<button type="button" class="text-xs bg-azure hover:bg-neon-blue px-3 py-1 rounded" onclick="setJobTemplate('service')">Service</button>
					<button type="button" class="text-xs border border-graphite px-3 py-1 rounded" onclick="setJobTemplate('native')">Native</button>
					<button type="button" class="text-xs border border-graphite px-3 py-1 rounded" onclick="setJobTemplate('qemu')">QEMU</button>
				</div>
			</div>
			<div>
				<label class="block text-sm text-gray-300 mb-1">Job TOML</label>
				<textarea id="job-toml" rows="15" class="w-full bg-graphite border border-graphite rounded px-3 py-2 text-sm font-mono" placeholder="Select a template above or enter custom TOML...">${examples['one-shot']}</textarea>
			</div>
			<div>
				<label class="block text-sm text-gray-300 mb-1">Attachments (optional)</label>
				<input id="job-assets" type="file" multiple class="w-full bg-graphite border border-graphite rounded px-3 py-2 text-sm" />
				<div id="job-assets-preview" class="mt-2 text-xs text-gray-300 space-y-1"></div>
				<p class="text-xs text-gray-400 mt-1">Files will be content-addressed and available as /tmp/assets/&lt;filename&gt; during job execution.</p>
			</div>
			<div class="flex items-center justify-end gap-2">
				<button type="button" class="border border-graphite px-4 py-2 rounded" id="job-cancel">Close</button>
				<button type="submit" class="bg-neon-blue hover:bg-azure px-4 py-2 rounded">Submit</button>
			</div>
		</form>`;
		
	// Make examples globally available for template buttons
	window.jobExamples = examples;
	showModal('Submit Job', body, null);
	const form = document.getElementById('job-submit-form');
	const closeBtn = document.getElementById('job-cancel');
	if (closeBtn) closeBtn.addEventListener('click', () => hideModal());
	if (form) {
		form.addEventListener('submit', async (e) => {
			e.preventDefault();
			const toml = document.getElementById('job-toml').value;
			const fd = new FormData(); fd.append('job_toml', toml);
			const filesInput = document.getElementById('job-assets');
			const files = filesInput.files;
			if (files && files.length) {
				for (let i = 0; i < files.length; i++) { fd.append('asset', files[i], files[i].name); }
			}
			// Client-side preview of pre-stage plan with sha256 digests
			try {
				const preview = document.getElementById('job-assets-preview');
				if (preview && files && files.length) {
					preview.innerHTML = '';
					for (let i = 0; i < files.length; i++) {
						const f = files[i];
						const ab = await f.arrayBuffer();
						const digest = await sha256Hex(new Uint8Array(ab));
						const div = document.createElement('div');
						div.className = 'font-mono';
						div.textContent = `${f.name} → /tmp/assets/${f.name}  sha256:${digest.slice(0, 16)}…`;
						preview.appendChild(div);
					}
				}
			} catch {}
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
		
		// Get artifacts
		let artifactsHtml = '';
		if (job.artifacts && job.artifacts.length > 0) {
			artifactsHtml = `
				<div><strong>Artifacts:</strong>
					<div class='mt-2 space-y-1'>
						${job.artifacts.map(artifact => `
							<div class='flex justify-between items-center p-2 bg-graphite rounded'>
								<span class='text-sm'>${artifact.name} (${artifact.size_bytes ? (artifact.size_bytes + ' bytes') : 'unknown size'})</span>
								<button class='text-azure hover:text-neon-blue text-sm' onclick="downloadArtifact('${jobId}', '${artifact.name}')">
									<i class="fa-solid fa-download mr-1"></i>Download
								</button>
							</div>
						`).join('')}
					</div>
				</div>`;
		}
		
		showModal('Job Details', `
			<div class='space-y-4'>
				<div><strong>ID:</strong> ${job.id}</div>
				<div><strong>Name:</strong> ${job.spec?.name || '-'}</div>
				<div><strong>Status:</strong> ${job.status}</div>
				<div><strong>Type:</strong> ${job.spec?.job_type || '-'}</div>
				<div><strong>Node:</strong> ${job.assigned_node || '-'}</div>
				${job.spec?.schedule ? `<div><strong>Schedule:</strong> ${job.spec.schedule}</div>` : ''}
				${artifactsHtml}
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

// Global function for artifact download
window.downloadArtifact = async function(jobId, artifactName) {
	try {
		const app = window.app; // Access the global app instance
		const response = await apiCall(app.sessionToken, `/api/jobs/${encodeURIComponent(jobId)}/artifacts/${encodeURIComponent(artifactName)}`);
		
		if (!response.ok) {
			throw new Error('Download failed');
		}
		
		const blob = await response.blob();
		const url = window.URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.style.display = 'none';
		a.href = url;
		a.download = artifactName;
		document.body.appendChild(a);
		a.click();
		window.URL.revokeObjectURL(url);
		document.body.removeChild(a);
		
		showSuccess(`Downloaded artifact: ${artifactName}`);
	} catch (e) {
		showError('Failed to download artifact');
		console.error('Download error:', e);
	}
};

// Global function for job template selection
window.setJobTemplate = function(templateType) {
	const textarea = document.getElementById('job-toml');
	if (textarea && window.jobExamples && window.jobExamples[templateType]) {
		textarea.value = window.jobExamples[templateType];
	}
};


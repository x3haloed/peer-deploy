export function showModal(title, body, onConfirm = null) {
	document.getElementById('modal-title').textContent = title;
	document.getElementById('modal-body').innerHTML = body;
	const overlay = document.getElementById('modal-overlay');
	overlay.classList.remove('hidden');
	overlay.style.display = 'flex';
	const confirmBtn = document.querySelector('.modal-confirm');
	if (onConfirm) {
		confirmBtn.style.display = 'block';
		confirmBtn.onclick = () => { onConfirm(); hideModal(); };
	} else {
		confirmBtn.style.display = 'none';
	}
}

export function hideModal() {
	const overlay = document.getElementById('modal-overlay');
	overlay.style.display = 'none';
	overlay.classList.add('hidden');
}

export function showSuccess(message) { showNotification(message, 'success'); }
export function showError(message) { showNotification(message, 'error'); }

export function showLoading(message) { console.log('Loading:', message); }
export function hideLoading() {}

export function showNotification(message, type) {
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
	setTimeout(() => { document.body.removeChild(notification); }, 5000);
}

export function addLogLine(timestamp, component, message, autoScroll = true) {
	const container = document.getElementById('logs-container');
	const line = document.createElement('div');
	line.className = 'flex gap-4 mb-1 whitespace-nowrap';
	line.innerHTML = `
		<span class="text-gray-400 min-w-[150px]">${timestamp}</span>
		<span class="text-yellow-400 min-w-[100px]">${component}</span>
		<span class="text-gray-100 whitespace-pre-wrap">${message}</span>
	`;
	container.appendChild(line);
	while (container.children.length > 1000) { container.removeChild(container.firstChild); }
	if (autoScroll) { container.scrollTop = container.scrollHeight; }
}

export function clearLogs() {
	document.getElementById('logs-container').innerHTML = '';
}

export function addActivity(message) {
	const container = document.getElementById('recent-activity');
	const item = document.createElement('div');
	item.className = 'activity-item';
	item.innerHTML = `
		<span class="activity-time">Just now</span>
		<span class="activity-desc">${message}</span>
	`;
	container.insertBefore(item, container.firstChild);
	while (container.children.length > 10) { container.removeChild(container.lastChild); }
}

// Compute sha256 hex in browser for preview
export async function sha256Hex(bytes) {
    if (window.crypto && window.crypto.subtle) {
        const digest = await window.crypto.subtle.digest('SHA-256', bytes);
        const arr = Array.from(new Uint8Array(digest));
        return arr.map(b => b.toString(16).padStart(2, '0')).join('');
    }
    return '';
}


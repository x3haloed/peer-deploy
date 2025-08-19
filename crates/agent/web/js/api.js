export async function apiCall(token, url, options = {}) {
	const defaultOptions = {
		headers: {
			'Authorization': `Bearer ${token}`,
			'Content-Type': 'application/json',
			...(options.headers || {})
		}
	};

	const response = await fetch(url, { ...defaultOptions, ...options });
	if (!response.ok) {
		throw new Error(`API call failed: ${response.status} ${response.statusText}`);
	}
	return response;
}


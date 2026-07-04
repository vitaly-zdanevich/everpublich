const removeButton = document.getElementById('remove-account');

removeButton.addEventListener('click', async function () {
	if (!confirm('Remove this Everpublich account?')) {
		return;
	}
	await fetch('/api/remove-account', {
		method: 'POST',
		headers: {
			'content-type': 'application/json',
			authorization: 'Bearer ' + (localStorage.getItem('everpublich_admin_token') || ''),
		},
		body: JSON.stringify({ user_id: localStorage.getItem('everpublich_user_id') }),
	});
	localStorage.removeItem('everpublich_admin_token');
	localStorage.removeItem('everpublich_user_id');
	location.href = '/';
});

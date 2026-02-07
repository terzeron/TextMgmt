export function determineRole(email, adminEmail, allowedEmails) {
    if (email === adminEmail) return 'admin';
    if (allowedEmails.includes(email)) return 'viewer';
    return null;
}

export function isViewerAllowedPath(pathname) {
    return pathname === '/' || pathname === '/view' || pathname.startsWith('/view/') || pathname.startsWith('/viewer/');
}

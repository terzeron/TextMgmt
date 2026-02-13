export function determineRole(email, adminEmail, allowedEmails) {
    if (email === adminEmail) return 'admin';
    if (allowedEmails.includes(email)) return 'viewer';
    return null;
}

export function isViewerAllowedPath(pathname) {
    return pathname === '/'
        || pathname === '/book-view' || pathname.startsWith('/book-view/')
        || pathname === '/comics-view' || pathname.startsWith('/comics-view/')
        || pathname.startsWith('/viewer/');
}

export function isViewerAllowedPath(pathname) {
    return pathname === '/'
        || pathname === '/book-view' || pathname.startsWith('/book-view/')
        || pathname === '/book-latest'
        || pathname === '/comics-view' || pathname.startsWith('/comics-view/')
        || pathname === '/comics-latest'
        || pathname.startsWith('/viewer/');
}

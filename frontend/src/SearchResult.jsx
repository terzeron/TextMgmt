import './Edit.css';
import './SearchResult.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense, useState, useEffect} from 'react';
import PropTypes from 'prop-types';

import {Card, Button} from 'react-bootstrap';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faChevronDown, faChevronRight} from "@fortawesome/free-solid-svg-icons";

import CoverGrid from './CoverGrid';

export default function SearchResult({results, role, showEditButton, onLoadMore, hasMore = false, loading = false, basePath = '/book-edit', title = '검색 결과', emptyMessage = '검색 결과가 없습니다.', viewMode = 'list', headerActions = null, categories = [], selectedCategory = '', onCategoryChange = null, categoryLoading = false}) {
    const [isOpen, setIsOpen] = useState(true);
    const canEdit = showEditButton !== undefined ? showEditButton : (role === 'admin');
    const availableResults = results || [];

    useEffect(() => {
        if (results && results.length > 0) {
            setIsOpen(true);
        }
    }, [results]);

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(!isOpen)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2 d-flex align-items-center">
                <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                {title}
                {/* 헤더 클릭은 접기/펼치기다. 헤더 안 조작 요소의 클릭이 거기로 번지지 않게 막는다. */}
                <div className="search-result-header-controls ms-auto" onClick={(e) => e.stopPropagation()}>
                    {headerActions}
                    {onCategoryChange && (
                        <select
                            aria-label="카테고리"
                            className="form-select form-select-sm search-result-category-select"
                            value={selectedCategory}
                            disabled={categoryLoading}
                            onChange={(event) => onCategoryChange(event.target.value)}
                        >
                            <option value="">전체 카테고리</option>
                            {categories.map((category) => (
                                <option key={category} value={category}>
                                    {category === '_root' ? '분류 없음' : category}
                                </option>
                            ))}
                        </select>
                    )}
                </div>
            </Card.Header>
            {isOpen && (
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {availableResults.length > 0 ? (
                        <>
                            {viewMode === 'cover' ? (
                                <CoverGrid results={availableResults} basePath={basePath || '/book-edit'}/>
                            ) : availableResults.map((book) => {
                                const filename = (book.file_path || '').split('/').pop() || book.title || 'Unknown';
                                const category = book.category || '_root';
                                const safeBasePath = basePath || '/book-edit';
                                const editBasePath = safeBasePath.replace('-view', '-edit');
                                const viewBasePath = safeBasePath.replace('-edit', '-view');
                                const filePathParam = encodeURIComponent(book.file_path || '');
                                const categoryParam = encodeURIComponent(category);
                                const fileType = book.file_type || 'epub';
                                const displayName = (!category || category === '_root') ? filename : `${category}/${filename}`;
                                return (
                                <div key={book.book_id} className="search-result-item">
                                    <span className="search-result-item-text">{displayName}</span>
                                    <div className="search-result-item-actions">
                                        {canEdit && (
                                            <Button
                                                variant="outline-warning" size="sm"
                                                onClick={() => window.open(`${editBasePath}/${book.book_id}?category=${categoryParam}`, '_blank', 'noopener')}
                                                style={{marginRight: '4px'}}
                                            >
                                                편집
                                            </Button>
                                        )}
                                        <Button
                                            variant="outline-primary" size="sm"
                                            onClick={() => window.open(`${viewBasePath}/${book.book_id}?category=${categoryParam}`, '_blank', 'noopener')}
                                            style={{marginRight: '4px'}}
                                        >
                                            조회
                                        </Button>
                                        <Button
                                            variant="outline-secondary" size="sm"
                                            onClick={() => {
                                                const apiParam = safeBasePath.startsWith('/comics') ? '&api=%2Fcomics' : '';
                                                window.open(`/viewer/${fileType}/${book.book_id}?path=${filePathParam}${apiParam}`, '_blank', 'noopener');
                                            }}
                                        >
                                            전체보기
                                        </Button>
                                    </div>
                                </div>
                                );
                            })}
                            {hasMore && (
                                <div className="search-result-load-more-wrapper">
                                    <div
                                        className={`search-result-load-more${loading ? ' disabled' : ''}`}
                                        onClick={loading ? undefined : onLoadMore}
                                        role="button"
                                        tabIndex={0}
                                        onKeyDown={(e) => { if (!loading && (e.key === 'Enter' || e.key === ' ')) onLoadMore(); }}
                                    >
                                        {loading ? '로딩 중...' : '더 보기'}
                                        <FontAwesomeIcon icon={faChevronDown} size="sm" />
                                    </div>
                                </div>
                            )}
                        </>
                    ) : (
                        <div>{emptyMessage}</div>
                    )}
                </Card.Body>
            </Suspense>
            )}
        </Card>
    );
}

SearchResult.propTypes = {
    results: PropTypes.array,
    role: PropTypes.string,
    showEditButton: PropTypes.bool,
    onLoadMore: PropTypes.func,
    hasMore: PropTypes.bool,
    loading: PropTypes.bool,
    basePath: PropTypes.string,
    title: PropTypes.string,
    emptyMessage: PropTypes.string,
    viewMode: PropTypes.oneOf(['list', 'cover']),
    headerActions: PropTypes.node,
    categories: PropTypes.arrayOf(PropTypes.string),
    selectedCategory: PropTypes.string,
    onCategoryChange: PropTypes.func,
    categoryLoading: PropTypes.bool,
};

SearchResult.defaultProps = {
    results: [],
    role: null,
    showEditButton: undefined,
    hasMore: false,
    loading: false,
    title: '검색 결과',
    emptyMessage: '검색 결과가 없습니다.'
};

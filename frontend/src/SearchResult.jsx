import './Edit.css';
import './SearchResult.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense, useState, useEffect} from 'react';
import PropTypes from 'prop-types';

import {Card, Button} from 'react-bootstrap';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faChevronDown, faChevronRight} from "@fortawesome/free-solid-svg-icons";

export default function SearchResult({results, showEditButton = true, onLoadMore, hasMore = false, loading = false, basePath = '/book-edit', title = '검색 결과', emptyMessage = '검색 결과가 없습니다.'}) {
    const [isOpen, setIsOpen] = useState(true);

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
                className="py-2">
                <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                {title}
            </Card.Header>
            {isOpen && (
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {results && results.length > 0 ? (
                        <>
                            {results.map((book) => {
                                const filename = (book.file_path || '').split('/').pop() || book.title || 'Unknown';
                                const category = book.category || '_root';
                                const safeBasePath = basePath || '/book-edit';
                                const viewBasePath = safeBasePath.replace('-edit', '-view');
                                const filePathParam = encodeURIComponent(book.file_path || '');
                                const categoryParam = encodeURIComponent(category);
                                const fileType = book.file_type || 'epub';
                                return (
                                <div key={book.book_id} className="search-result-item">
                                    <span className="search-result-item-text">{category}/{filename}</span>
                                    <div className="search-result-item-actions">
                                        {showEditButton && (
                                            <Button
                                                variant="outline-warning" size="sm"
                                                onClick={() => window.open(`${safeBasePath}/${book.book_id}?category=${categoryParam}`, '_blank', 'noopener')}
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
                                        {showEditButton || (
                                            <Button
                                                variant="outline-secondary" size="sm"
                                                onClick={() => {
                                                    const apiParam = safeBasePath.startsWith('/comics') ? '&api=%2Fcomics' : '';
                                                    window.open(`/viewer/${fileType}/${book.book_id}?path=${filePathParam}${apiParam}`, '_blank', 'noopener');
                                                }}
                                            >
                                                전체 보기
                                            </Button>
                                        )}
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
    showEditButton: PropTypes.bool,
    onLoadMore: PropTypes.func,
    hasMore: PropTypes.bool,
    loading: PropTypes.bool,
    basePath: PropTypes.string,
    title: PropTypes.string,
    emptyMessage: PropTypes.string,
};

SearchResult.defaultProps = {
    results: [],
    showEditButton: true,
    hasMore: false,
    loading: false,
    title: '검색 결과',
    emptyMessage: '검색 결과가 없습니다.'
};

import './Edit.css';
import './SearchResult.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense, useState, useEffect} from 'react';
import PropTypes from 'prop-types';

import {Card, Button} from 'react-bootstrap';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faChevronDown, faChevronRight} from "@fortawesome/free-solid-svg-icons";

export default function SearchResult({results, showEditButton = true, onLoadMore, hasMore = false, loading = false}) {
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
                검색 결과
            </Card.Header>
            {isOpen && (
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {results && results.length > 0 ? (
                        <>
                            {results.map((book) => (
                                <div key={book.book_id} style={{padding: '4px', borderBottom: '1px solid #eee', display: 'flex', alignItems: 'center', justifyContent: 'space-between'}}>
                                    <span>{book.category}/{book.file_path.split('/').pop()}</span>
                                    <div>
                                        {showEditButton && (
                                            <Button
                                                variant="outline-warning" size="sm"
                                                onClick={() => window.open(`/edit/${book.category.split('/').map(encodeURIComponent).join('/')}/${book.book_id}`, '_blank', 'noopener')}
                                                style={{marginRight: '4px'}}
                                            >
                                                편집
                                            </Button>
                                        )}
                                        <Button
                                            variant="outline-primary" size="sm"
                                            onClick={() => window.open(`/view/${book.category.split('/').map(encodeURIComponent).join('/')}/${book.book_id}`, '_blank', 'noopener')}
                                            style={{marginRight: '4px'}}
                                        >
                                            조회
                                        </Button>
                                        {showEditButton || (
                                            <Button
                                                variant="outline-secondary" size="sm"
                                                onClick={() => window.open(`/viewer/${book.file_type}/${book.book_id}?path=${encodeURIComponent(book.file_path)}`, '_blank', 'noopener')}
                                            >
                                                전체 보기
                                            </Button>
                                        )}
                                    </div>
                                </div>
                            ))}
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
                        <div>검색 결과가 없습니다.</div>
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
    loading: PropTypes.bool
};

SearchResult.defaultProps = {
    results: [],
    showEditButton: true,
    hasMore: false,
    loading: false
};

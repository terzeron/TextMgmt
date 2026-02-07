import './Edit.css';
import './SearchResult.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {useEffect, useState, useCallback, Suspense} from 'react';
import PropTypes from 'prop-types';
import {rawJsonGetReq} from './Common';

import {Card, Button} from 'react-bootstrap';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faChevronDown, faChevronRight} from "@fortawesome/free-solid-svg-icons";

export default function SimilarBooks({bookId, onSelect}) {
    const [isOpen, setIsOpen] = useState(false);
    const [similarBooks, setSimilarBooks] = useState([]);
    const [total, setTotal] = useState(0);
    const [loadingMore, setLoadingMore] = useState(false);

    useEffect(() => {
        if (bookId) {
            rawJsonGetReq(`/similar/${bookId}?offset=0&limit=10`, (data) => {
                if (data.status === 'success') {
                    setSimilarBooks(data.result || []);
                    setTotal(data.total || 0);
                }
            }, (error) => {
                console.error(error);
            });
        }
        return () => {
            setSimilarBooks([]);
            setTotal(0);
        };
    }, [bookId]);

    const handleLoadMore = useCallback(() => {
        if (loadingMore) return;
        setLoadingMore(true);
        const offset = similarBooks.length;
        rawJsonGetReq(`/similar/${bookId}?offset=${offset}&limit=10`, (data) => {
            if (data.status === 'success' && data.result) {
                setSimilarBooks(prev => [...prev, ...data.result]);
                setTotal(data.total || 0);
            }
            setLoadingMore(false);
        }, (error) => {
            console.error(error);
            setLoadingMore(false);
        });
    }, [bookId, similarBooks.length, loadingMore]);

    const hasMore = similarBooks.length < total;

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(!isOpen)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} className="me-2"/>
                유사한 책 목록
            </Card.Header>
            {isOpen &&
                <Card.Body>
                    {similarBooks && similarBooks.length > 0 ? (
                        <>
                            {similarBooks.map((book) => (
                                <div key={book.book_id} style={{padding: '4px', borderBottom: '1px solid #eee', display: 'flex', alignItems: 'center', justifyContent: 'space-between'}}>
                                    <span style={{cursor: 'pointer'}} onClick={() => onSelect && onSelect(`${book.category}/${book.book_id}`)}>
                                        {book.file_path.split('/').pop()}
                                    </span>
                                    <div style={{whiteSpace: 'nowrap', flexShrink: 0}}>
                                        <Button
                                            variant="outline-warning" className="btn-xs"
                                            onClick={() => window.open(`/edit/${book.category}/${book.book_id}`, '_blank', 'noopener')}
                                            style={{marginRight: '4px'}}
                                        >
                                            편집
                                        </Button>
                                        <Button
                                            variant="outline-primary" className="btn-xs"
                                            onClick={() => window.open(`/view/${book.category}/${book.book_id}`, '_blank', 'noopener')}
                                            style={{marginRight: '4px'}}
                                        >
                                            조회
                                        </Button>
                                    </div>
                                </div>
                            ))}
                            {hasMore && (
                                <div className="search-result-load-more-wrapper">
                                    <div
                                        className={`search-result-load-more${loadingMore ? ' disabled' : ''}`}
                                        onClick={loadingMore ? undefined : handleLoadMore}
                                        role="button"
                                        tabIndex={0}
                                        onKeyDown={(e) => { if (!loadingMore && (e.key === 'Enter' || e.key === ' ')) handleLoadMore(); }}
                                    >
                                        {loadingMore ? '로딩 중...' : '더 보기'}
                                        <FontAwesomeIcon icon={faChevronDown} size="sm" />
                                    </div>
                                </div>
                            )}
                        </>
                    ) : (
                        <div>유사한 책이 없습니다.</div>
                    )}
                </Card.Body>
            }
        </Card>
    );
}

SimilarBooks.propTypes = {
    bookId: PropTypes.oneOfType([PropTypes.number, PropTypes.string]),
    onSelect: PropTypes.func
};

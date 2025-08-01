import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Suspense} from 'react';
import {useNavigate} from 'react-router-dom';
import PropTypes from 'prop-types';

import {Card, Button} from 'react-bootstrap';

export default function SearchResult({results, showEditButton = true}) {
    const navigate = useNavigate();
    return (
        <Card>
            <Card.Header>
                검색 결과
            </Card.Header>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                <Card.Body>
                    {results && results.length > 0 ? (
                        results.map((book) => (
                            <div key={book.book_id} style={{padding: '4px', borderBottom: '1px solid #eee', display: 'flex', alignItems: 'center', justifyContent: 'space-between'}}>
                                <span>{book.title} - {book.author}</span>
                                <div>
                                    {showEditButton && (
                                        <Button
                                            variant="outline-warning" size="sm"
                                            onClick={() => window.open(`/edit/${book.category}/${book.book_id}`, '_blank', 'noopener')}
                                            style={{marginRight: '4px'}}
                                        >
                                            편집
                                        </Button>
                                    )}
                                    <Button
                                        variant="outline-primary" size="sm"
                                        onClick={() => window.open(`/view/${book.category}/${book.book_id}`, '_blank', 'noopener')}
                                        style={{marginRight: '4px'}}
                                    >
                                        조회
                                    </Button>
                                    {showEditButton || (
                                        <Button
                                                variant="outline-secondary" size="sm"
                                                onClick={() => window.open(`/view/${book.file_type}/${book.book_id}/${encodeURIComponent(book.file_path)}`, '_blank', 'noopener')}
                                            >
                                            새 창에서 전체 보기
                                        </Button>
                                    )}
                                </div>
                            </div>
                        ))
                    ) : (
                        <div>검색 결과가 없습니다.</div>
                    )}
                </Card.Body>
            </Suspense>
        </Card>
    );
}

SearchResult.propTypes = {
    results: PropTypes.array,
    showEditButton: PropTypes.bool
};

SearchResult.defaultProps = {
    results: [],
    showEditButton: true
};

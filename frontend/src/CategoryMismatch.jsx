import {useEffect, useState, useCallback, useMemo} from 'react';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Card, Spinner} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faChevronDown, faChevronRight} from '@fortawesome/free-solid-svg-icons';

import {RichTreeView} from '@mui/x-tree-view/RichTreeView';

import {jsonGetReq} from './Common';
import {CustomTreeItem} from './Folder';
import {findCommonPrefix, buildFolderHierarchy} from './folderUtils';

function buildMismatchCounts(mismatchData) {
    const counts = {};
    for (const item of mismatchData.mismatches || []) {
        counts[item.category] = 1;
    }
    for (const item of mismatchData.es_only || []) {
        counts[item.category] = 1;
    }
    for (const item of mismatchData.fs_only || []) {
        counts[item.category] = 1;
    }
    return counts;
}

export default function CategoryMismatch() {
    const [folderData, setFolderData] = useState([]);
    const [totalMismatchCount, setTotalMismatchCount] = useState(0);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState('');
    const [isOpen, setIsOpen] = useState(false);
    const [expandedItems, setExpandedItems] = useState([]);

    const treeViewStyles = useMemo(() => ({
        height: 'fit-content',
        flexGrow: 1,
        maxWidth: 600,
        overflowY: 'auto',
    }), []);

    const loadData = useCallback(() => {
        setLoading(true);
        setError('');

        let categoriesResult = null;
        let mismatchResult = null;
        let completed = 0;
        let hasError = false;

        const tryBuild = () => {
            completed++;
            if (completed < 2 || hasError) return;

            const mismatchCounts = buildMismatchCounts(mismatchResult);

            // fs_only 카테고리는 ES에 없을 수 있으므로 병합
            const esCategories = Object.keys(categoriesResult).filter(c => c !== '_root');
            const fsOnlyCategories = (mismatchResult.fs_only || []).map(item => item.category);
            const allCategories = [...new Set([...esCategories, ...fsOnlyCategories])].sort((a, b) => a.localeCompare(b));

            const commonPrefix = findCommonPrefix(allCategories);
            const data = buildFolderHierarchy(allCategories, commonPrefix, mismatchCounts);

            const total = Object.keys(mismatchCounts).length;
            setTotalMismatchCount(total);
            setFolderData(data);
            setExpandedItems(data.filter(d => d.count > 0).map(d => d.id));
            setLoading(false);
        };

        jsonGetReq('/categories', null,
            (result) => { categoriesResult = result; tryBuild(); },
            (err) => { hasError = true; setError(`카테고리 목록을 불러올 수 없습니다. ${err}`); setLoading(false); }
        );

        jsonGetReq('/category-mismatches', null,
            (result) => { mismatchResult = result; tryBuild(); },
            (err) => { hasError = true; setError(`불일치 데이터를 불러올 수 없습니다. ${err}`); setLoading(false); }
        );
    }, []);

    useEffect(() => {
        loadData();
    }, [loadData]);

    if (!isOpen) {
        return (
            <Card className="mt-3">
                <Card.Header
                    onClick={() => setIsOpen(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    불일치 관리
                    {totalMismatchCount > 0 && <span className="text-muted ms-2">({totalMismatchCount}건)</span>}
                </Card.Header>
            </Card>
        );
    }

    return (
        <Card className="mt-3">
            <Card.Header
                onClick={() => setIsOpen(false)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={faChevronDown} className="me-2"/>
                불일치 관리
                {totalMismatchCount > 0 && <span className="text-muted ms-2">({totalMismatchCount}건)</span>}
            </Card.Header>
            <Card.Body>
                {error && (
                    <div className="alert alert-danger py-1 mb-2">{error}</div>
                )}
                {loading ? (
                    <div className="text-center p-4">
                        <Spinner animation="border"/>
                        <p className="mt-2">로딩 중...</p>
                    </div>
                ) : folderData.length === 0 ? (
                    <div className="text-muted p-3">불일치 없음</div>
                ) : (
                    <RichTreeView
                        items={folderData}
                        aria-label="category mismatches"
                        sx={treeViewStyles}
                        slots={{item: CustomTreeItem}}
                        expandedItems={expandedItems}
                        onExpandedItemsChange={(event, newExpandedItems) => setExpandedItems(newExpandedItems)}
                    />
                )}
            </Card.Body>
        </Card>
    );
}

import {useEffect, useState, useCallback, useRef, useMemo} from 'react';
import React from 'react';
import PropTypes from 'prop-types';

import 'bootstrap/dist/css/bootstrap.min.css';
import {Button, Card, Form, InputGroup, Badge, Row, Col, Spinner, Modal} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faPlus, faTrash, faEyeSlash, faChevronDown, faChevronRight, faEdit, faRotate} from '@fortawesome/free-solid-svg-icons';

import {RichTreeView} from '@mui/x-tree-view/RichTreeView';
import {TreeItem2Content, TreeItem2IconContainer, TreeItem2Label, TreeItem2Root} from '@mui/x-tree-view/TreeItem2';
import {TreeItem2Icon} from '@mui/x-tree-view/TreeItem2Icon';
import {TreeItem2Provider} from '@mui/x-tree-view/TreeItem2Provider';
import {unstable_useTreeItem2 as useTreeItem2} from '@mui/x-tree-view/useTreeItem2';
import {treeItemClasses} from '@mui/x-tree-view/TreeItem';
import {styled, alpha} from '@mui/material/styles';
import {animated, useSpring} from '@react-spring/web';
import Collapse from '@mui/material/Collapse';
import Typography from '@mui/material/Typography';
import Box from '@mui/material/Box';
import FolderRounded from '@mui/icons-material/FolderRounded';
import clsx from 'clsx';

import {jsonGetReq, jsonPostReq, jsonPutReq, jsonDeleteReq} from './Common';
import {fetchCategoryMappings, updateCachedMappings} from './categoryMappingCache';
import {findCommonPrefix, buildFolderHierarchy, findFolderInTree, updateFolderInTree} from './folderUtils';
import './Folder.css';

// ── MUI TreeItem 스타일 (Folder.jsx의 CustomTreeItem 스타일 재사용) ──

const StyledTreeItemRoot = styled(TreeItem2Root)(({theme}) => ({
    color: theme.palette.mode === 'light' ? theme.palette.grey[800] : theme.palette.grey[400],
    position: 'relative',
    [`& .${treeItemClasses.groupTransition}`]: {
        marginLeft: theme.spacing(3.5),
    },
}));

const CustomTreeItemContent = styled(TreeItem2Content)(({theme}) => ({
    flexDirection: 'row-reverse',
    borderRadius: theme.spacing(0.7),
    marginBottom: theme.spacing(0.1),
    marginTop: theme.spacing(0.1),
    padding: theme.spacing(0.1),
    paddingRight: theme.spacing(0.2),
    fontWeight: 400,
    [`& .${treeItemClasses.iconContainer}`]: {
        marginRight: theme.spacing(2),
    },
    [`&.Mui-expanded `]: {
        '&:not(.Mui-focused, .Mui-selected, .Mui-selected.Mui-focused) .labelIcon': {
            color: theme.palette.mode === 'light' ? theme.palette.primary.main : theme.palette.primary.dark,
        },
        '&::before': {
            content: '""',
            display: 'block',
            position: 'absolute',
            left: '16px',
            top: '44px',
            height: 'calc(100% - 48px)',
            width: '1.5px',
            backgroundColor: theme.palette.mode === 'light' ? theme.palette.grey[300] : theme.palette.grey[700],
        },
    },
    '&:hover': {
        backgroundColor: alpha(theme.palette.primary.main, 0.1),
        color: theme.palette.mode === 'light' ? theme.palette.primary.main : 'white',
    },
    [`&.Mui-focused, &.Mui-selected, &.Mui-selected.Mui-focused`]: {
        backgroundColor: theme.palette.mode === 'light' ? theme.palette.primary.main : theme.palette.primary.dark,
        color: theme.palette.primary.contrastText,
    },
}));

const AnimatedCollapse = animated(Collapse);

function TransitionComponent(props) {
    const style = useSpring({
        to: {
            // eslint-disable-next-line react/prop-types
            opacity: props.in ? 1 : 0,
            // eslint-disable-next-line react/prop-types
            transform: `translate3d(0,${props.in ? 0 : 20}px,0)`,
        },
    });
    return <AnimatedCollapse style={style} {...props} />;
}

const StyledTreeItemLabelText = styled(Typography)({
    color: 'inherit',
    fontFamily: 'General Sans',
    fontWeight: 500,
});

function DotIcon() {
    return (
        <Box sx={{width: 6, height: 6, borderRadius: '70%', bgcolor: 'warning.main', display: 'inline-block', verticalAlign: 'middle', zIndex: 1, mx: 1}} />
    );
}

// eslint-disable-next-line react/prop-types
function AdminLabel({icon: Icon, iconColor, expandable, count, children, ...other}) {
    return (
        <TreeItem2Label
            {...other}
            sx={{display: 'flex', alignItems: 'center', width: '100%', overflow: 'hidden'}}
        >
            {Icon && <Box component={Icon} className="labelIcon" sx={{mr: 1, fontSize: '1.2rem', color: iconColor || 'inherit'}} />}
            <StyledTreeItemLabelText variant="body2" sx={{flex: '1 1 0%', minWidth: 0, wordBreak: 'break-word'}}>{children}</StyledTreeItemLabelText>
            {count > 0 && (
                <Typography variant="caption" component="span" sx={{color: 'text.secondary', fontWeight: 400, fontSize: '0.45rem', flexShrink: 0, whiteSpace: 'nowrap', ml: 'auto', textAlign: 'right'}}>
                    {count}
                </Typography>
            )}
            {expandable && <DotIcon />}
        </TreeItem2Label>
    );
}

const isExpandable = (reactChildren) => {
    if (Array.isArray(reactChildren)) {
        return reactChildren.length > 0 && reactChildren.some(isExpandable);
    }
    return Boolean(reactChildren);
};

// AdminTreeItem: hidden 카테고리에 opacity 적용하는 래퍼
const AdminTreeItem = React.forwardRef(function AdminTreeItem(props, ref) {
    // eslint-disable-next-line react/prop-types
    const {id, itemId, label, disabled, children, ...other} = props;

    const {
        getRootProps,
        getContentProps,
        getIconContainerProps,
        getLabelProps,
        getGroupTransitionProps,
        status,
        publicAPI,
    } = useTreeItem2({id, itemId, children, label, disabled, rootRef: ref});

    const item = useMemo(() => publicAPI.getItem(itemId), [publicAPI, itemId]);
    const expandable = isExpandable(children);
    const icon = FolderRounded;
    const iconColor = item?.isHidden ? '#9e9e9e' : '#ffc107';
    const opacity = item?.isHidden ? 0.5 : 1;

    return (
        <TreeItem2Provider itemId={itemId}>
            <StyledTreeItemRoot {...getRootProps(other)} style={{opacity}}>
                <CustomTreeItemContent
                    {...getContentProps({
                        className: clsx('content', {
                            'Mui-expanded': status.expanded,
                            'Mui-selected': status.selected,
                            'Mui-focused': status.focused,
                            'Mui-disabled': status.disabled,
                        }),
                    })}
                >
                    <TreeItem2IconContainer {...getIconContainerProps()}>
                        <TreeItem2Icon status={status} />
                    </TreeItem2IconContainer>
                    <AdminLabel
                        {...getLabelProps({icon, iconColor, expandable: expandable && status.expanded, count: item?.count})}
                    />
                </CustomTreeItemContent>
                {children && <TransitionComponent {...getGroupTransitionProps()} />}
            </StyledTreeItemRoot>
        </TreeItem2Provider>
    );
});

// ── 불일치 건수 계산 ──

function buildMismatchCounts(mismatchData) {
    const counts = {};
    for (const item of mismatchData.mismatches || []) {
        counts[item.category] = Math.abs(item.diff);
    }
    for (const item of mismatchData.es_only || []) {
        counts[item.category] = item.es_count;
    }
    for (const item of mismatchData.fs_only || []) {
        counts[item.category] = item.fs_count;
    }
    return counts;
}

// ── 메인 컴포넌트 ──

export default function CategoryAdmin({contentType = 'book', title = '카테고리 관리'}) {
    // 공통 상태
    const [loading, setLoading] = useState(false);
    const [message, setMessage] = useState('');
    const [saving, setSaving] = useState(false);
    const [isOpen, setIsOpen] = useState(false);

    // 카테고리 트리
    const [folderData, setFolderData] = useState([]);
    const [expandedItems, setExpandedItems] = useState([]);
    const [hiddenCategories, setHiddenCategories] = useState(new Set());

    // 선택 상태
    const [selectedCategory, setSelectedCategory] = useState('');
    const [selectedMismatch, setSelectedMismatch] = useState(null);
    const [actionResult, setActionResult] = useState(null);

    // 카테고리 관리 (키워드)
    const [mappings, setMappings] = useState({});
    const [newKeyword, setNewKeyword] = useState('');
    const keywordInputRef = useRef(null);

    // rename/delete 모달
    const [showRenameModal, setShowRenameModal] = useState(false);
    const [showDeleteModal, setShowDeleteModal] = useState(false);
    const [showReloadModal, setShowReloadModal] = useState(false);
    const [reloading, setReloading] = useState(false);
    const [newCategoryName, setNewCategoryName] = useState('');

    const apiPrefix = contentType === 'comic' ? '/comics' : '';
    const contentLabel = contentType === 'comic' ? '만화' : '책';

    // ── 데이터 로드 ──

    const loadData = useCallback(() => {
        setLoading(true);
        setMessage('');

        let categoriesResult = null;
        let mismatchResult = null;
        let mappingsResult = null;
        let hiddenResult = null;
        let completed = 0;
        let hasError = false;
        const total = 4;

        const tryBuild = () => {
            completed++;
            if (completed < total || hasError) return;

            // 매핑 캐시 갱신
            setMappings(mappingsResult || {});
            updateCachedMappings(contentType, mappingsResult || {});

            // 비노출 카테고리 설정
            setHiddenCategories(new Set(hiddenResult || []));

            // 불일치 건수
            const mismatchCounts = buildMismatchCounts(mismatchResult);

            // 모든 카테고리 목록
            const esCategories = Object.keys(categoriesResult).filter(c => c !== '_root');
            const fsOnlyCategories = (mismatchResult.fs_only || []).map(item => item.category);
            const allCategories = [...new Set([...esCategories, ...fsOnlyCategories])].sort((a, b) => a.localeCompare(b));

            const commonPrefix = findCommonPrefix(allCategories);

            // 트리 빌드: 모든 카테고리 표시, 불일치 건수 포함
            const categoryCounts = {};
            for (const cat of allCategories) {
                categoryCounts[cat] = mismatchCounts[cat] || 0;
            }
            const data = buildFolderHierarchy(allCategories, commonPrefix, categoryCounts);

            // 비노출 카테고리에 isHidden 플래그 설정 + 불일치 있는 leaf에 placeholder child
            const hiddenSet = new Set(hiddenResult || []);
            const enriched = data.map(item => {
                const enrichItem = (node) => {
                    const enriched = {...node, isHidden: hiddenSet.has(node.id)};
                    if (enriched.children) {
                        enriched.children = enriched.children.map(enrichItem);
                    }
                    // 불일치가 있는 leaf 카테고리에 placeholder child 추가 (확장 아이콘 표시용)
                    if (enriched.count > 0 && !enriched.children?.length && !enriched.isVirtualParent) {
                        enriched.children = [{id: enriched.id + '/__placeholder__', label: '로딩 중...', fileType: 'placeholder'}];
                    }
                    return enriched;
                };
                return enrichItem(item);
            });

            setFolderData(enriched);
            setExpandedItems([]);
            setLoading(false);
        };

        // 1) 카테고리 목록
        jsonGetReq(apiPrefix + '/categories', null,
            (result) => { categoriesResult = result; tryBuild(); },
            (err) => { hasError = true; setMessage(`카테고리 목록을 불러올 수 없습니다. ${err}`); setLoading(false); }
        );

        // 2) 불일치 데이터
        jsonGetReq(apiPrefix + '/category-mismatches', null,
            (result) => { mismatchResult = result; tryBuild(); },
            (err) => { hasError = true; setMessage(`불일치 데이터를 불러올 수 없습니다. ${err}`); setLoading(false); }
        );

        // 3) 키워드 매핑
        jsonGetReq(`/category-mappings?content_type=${contentType}`, null,
            (result) => { mappingsResult = result; tryBuild(); },
            () => { mappingsResult = {}; tryBuild(); }
        );

        // 4) 비노출 카테고리
        jsonGetReq(`/hidden-categories?content_type=${contentType}`, null,
            (result) => { hiddenResult = result; tryBuild(); },
            () => { hiddenResult = []; tryBuild(); }
        );
    }, [apiPrefix, contentType]);

    useEffect(() => {
        loadData();
    }, [loadData]);

    // ── 폴더 클릭 → 불일치 detail lazy-load ──

    const onFolderClick = useCallback((selectedId) => {
        const selectedFolderData = findFolderInTree(folderData, selectedId);
        if (!selectedFolderData || selectedFolderData.fileType !== 'folder') return;
        if (selectedFolderData.isVirtualParent) return;
        if (!selectedFolderData.count) return;
        if (selectedFolderData.booksLoaded) return;

        jsonGetReq(apiPrefix + '/category-mismatches/' + selectedId, null,
            (result) => {
                const entries = [];

                for (const item of result.es_only || []) {
                    entries.push({
                        id: selectedId + '/es_' + item.book_id.toString(),
                        label: item.title + '.' + item.file_type,
                        fileType: item.file_type,
                        children: [],
                        mismatchType: 'es_only',
                        bookId: item.book_id,
                        category: selectedId,
                        filePath: item.file_path,
                    });
                }

                for (const item of result.fs_only || []) {
                    entries.push({
                        id: selectedId + '/fs_' + item.file_name,
                        label: item.file_name,
                        fileType: 'unknown',
                        children: [],
                        mismatchType: 'fs_only',
                        category: selectedId,
                        filePath: item.file_path,
                    });
                }

                for (const item of result.duplicates || []) {
                    const ids = item.docs.map(d => d.book_id);
                    const fileName = item.file_path.split('/').pop();
                    entries.push({
                        id: selectedId + '/dup_' + ids.join('_'),
                        label: `[중복] ${fileName} (${item.docs.length}건)`,
                        fileType: item.docs[0]?.file_type || 'unknown',
                        children: [],
                        mismatchType: 'duplicate',
                        dupDocs: item.docs,
                        category: selectedId,
                        filePath: item.file_path,
                    });
                }

                const data = updateFolderInTree(folderData, selectedId, (folder) => {
                    const existingSubfolders = (folder.children || []).filter(c => c.fileType === 'folder');
                    return {
                        ...folder,
                        booksLoaded: true,
                        children: [...existingSubfolders, ...entries],
                    };
                });
                setFolderData(data);
            },
            (error) => {
                setMessage(error || '불일치 상세 조회에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            }
        );
    }, [folderData, apiPrefix]);

    // ── 트리 아이템 클릭 핸들러 ──

    const handleTreeItemClick = useCallback((event, selectedId) => {
        // 불일치 항목(leaf) 클릭 확인 - 재귀 검색
        const findMismatchItem = (items) => {
            for (const item of items) {
                if (item.id === selectedId && item.mismatchType) return item;
                if (item.children) {
                    const found = findMismatchItem(item.children);
                    if (found) return found;
                }
            }
            return null;
        };

        const foundMismatch = findMismatchItem(folderData);

        if (foundMismatch) {
            setSelectedMismatch(foundMismatch);
            setSelectedCategory('');
            setActionResult(null);
            return;
        }

        // placeholder 클릭 무시
        const foundFolder = findFolderInTree(folderData, selectedId);
        if (!foundFolder) return;
        if (foundFolder.fileType === 'placeholder') return;

        // 폴더 클릭 → 카테고리 선택 + expand 토글 + 불일치 detail lazy-load
        setSelectedMismatch(null);
        setActionResult(null);
        setSelectedCategory(selectedId);
        setNewKeyword('');

        const willExpand = !expandedItems.includes(selectedId);
        setExpandedItems((prev) =>
            willExpand ? [...prev, selectedId] : prev.filter(x => x !== selectedId)
        );
        if (willExpand) {
            onFolderClick(selectedId);
        }
    }, [folderData, expandedItems, onFolderClick]);

    // ── 카테고리 관리 핸들러 ──

    const handleAddKeyword = useCallback(() => {
        if (!selectedCategory || !newKeyword.trim()) return;
        const keyword = newKeyword.trim();

        if (mappings[selectedCategory]?.includes(keyword)) {
            setMessage('이미 등록된 키워드입니다.');
            setTimeout(() => setMessage(''), 3000);
            return;
        }

        setSaving(true);
        jsonPostReq(
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords?content_type=${contentType}`,
            {keyword},
            () => {
                setMappings(prev => {
                    const updated = {...prev};
                    if (!updated[selectedCategory]) updated[selectedCategory] = [];
                    updated[selectedCategory] = [...updated[selectedCategory], keyword];
                    updateCachedMappings(contentType, updated);
                    return updated;
                });
                setNewKeyword('');
                setTimeout(() => keywordInputRef.current?.focus(), 0);
            },
            (error) => {
                setMessage(error || '이미 등록된 키워드이거나 추가에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, newKeyword, mappings, contentType]);

    const handleRemoveKeyword = useCallback((keyword) => {
        if (!selectedCategory) return;
        setSaving(true);
        jsonDeleteReq(
            `/category-mappings/${encodeURIComponent(selectedCategory)}/keywords/${encodeURIComponent(keyword)}?content_type=${contentType}`,
            null,
            () => {
                setMappings(prev => {
                    const updated = {...prev};
                    if (updated[selectedCategory]) {
                        updated[selectedCategory] = updated[selectedCategory].filter(k => k !== keyword);
                    }
                    updateCachedMappings(contentType, updated);
                    return updated;
                });
            },
            (error) => {
                setMessage(error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, contentType]);

    const handleToggleHidden = useCallback((category, currentlyHidden) => {
        setSaving(true);
        jsonPostReq(
            `/hidden-categories/${category.split('/').map(encodeURIComponent).join('/')}?content_type=${contentType}`,
            {hidden: !currentlyHidden},
            (result) => {
                const newHidden = new Set(result || []);
                setHiddenCategories(newHidden);
                // 트리에서 isHidden 플래그 업데이트
                setFolderData(prev => {
                    const updateHidden = (items) => items.map(item => {
                        const updated = {...item, isHidden: newHidden.has(item.id)};
                        if (updated.children) {
                            updated.children = updateHidden(updated.children);
                        }
                        return updated;
                    });
                    return updateHidden(prev);
                });
            },
            (error) => {
                setMessage(error || '비노출 설정 변경에 실패했습니다.');
                setTimeout(() => setMessage(''), 3000);
            },
            () => setSaving(false)
        );
    }, [contentType]);

    const handleRenameCategory = useCallback(() => {
        if (!selectedCategory || !newCategoryName.trim()) return;
        const trimmed = newCategoryName.trim();
        if (trimmed === selectedCategory) {
            setMessage('현재 이름과 동일합니다.');
            setTimeout(() => setMessage(''), 3000);
            return;
        }

        setSaving(true);
        jsonPutReq(
            `${apiPrefix}/categories/rename`,
            {old_category: selectedCategory, new_category: trimmed},
            () => {
                setMessage(`카테고리 '${selectedCategory}'을(를) '${trimmed}'(으)로 변경했습니다.`);
                setTimeout(() => setMessage(''), 5000);
                setShowRenameModal(false);
                setSelectedCategory('');
                loadData();
            },
            (error) => {
                setMessage(error || '이름 변경에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, newCategoryName, apiPrefix, loadData]);

    const handleDeleteCategory = useCallback(() => {
        if (!selectedCategory) return;
        setSaving(true);
        jsonPostReq(
            `${apiPrefix}/categories/delete`,
            {category: selectedCategory},
            (result) => {
                setMessage(`카테고리 '${selectedCategory}'이(가) 삭제되었습니다. (${result.deleted_count}건)`);
                setTimeout(() => setMessage(''), 5000);
                setShowDeleteModal(false);
                setSelectedCategory('');
                loadData();
            },
            (error) => {
                setMessage(error || '삭제에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            },
            () => setSaving(false)
        );
    }, [selectedCategory, apiPrefix, loadData]);

    const handleReloadCategory = useCallback(() => {
        if (!selectedCategory) return;
        setShowReloadModal(false);
        setReloading(true);
        setSaving(true);
        jsonPostReq(
            `${apiPrefix}/category-mismatches/reload`,
            {category: selectedCategory},
            (result) => {
                setMessage(`카테고리 '${selectedCategory}' ES 재적재 완료 (${result.processed_count}건 처리)`);
                setTimeout(() => setMessage(''), 5000);
            },
            (error) => {
                setMessage(error || 'ES 재적재에 실패했습니다.');
                setTimeout(() => setMessage(''), 5000);
            },
            () => {
                setReloading(false);
                setSaving(false);
            }
        );
    }, [selectedCategory, apiPrefix]);

    // ── 불일치 관리 핸들러 ──

    const handleDeleteEsDoc = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'es_only') return;
        setActionResult(null);
        jsonDeleteReq(apiPrefix + '/books/' + selectedMismatch.bookId, null,
            (result) => {
                const warning = result?.warning;
                const msg = warning ? `${contentLabel} 정보가 삭제되었습니다. (${warning})` : `${contentLabel} 정보가 삭제되었습니다.`;
                setActionResult({type: 'success', message: msg});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);
                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `삭제 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData, apiPrefix, contentLabel]);

    const handleIndexFile = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'fs_only') return;
        setActionResult(null);
        jsonPostReq(apiPrefix + '/category-mismatches/index-file', {file_path: selectedMismatch.filePath},
            () => {
                setActionResult({type: 'success', message: 'ES에 적재되었습니다.'});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);
                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `ES 적재 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData, apiPrefix]);

    const handleDeleteFile = useCallback(() => {
        if (!selectedMismatch || selectedMismatch.mismatchType !== 'fs_only') return;
        setActionResult(null);
        jsonPostReq(apiPrefix + '/category-mismatches/delete-file', {file_path: selectedMismatch.filePath},
            () => {
                setActionResult({type: 'success', message: '파일이 삭제되었습니다.'});
                const data = updateFolderInTree(folderData, selectedMismatch.category, (folder) => ({
                    ...folder,
                    children: (folder.children || []).filter(c => c.id !== selectedMismatch.id),
                }));
                setFolderData(data);
                setSelectedMismatch(null);
            },
            (error) => {
                setActionResult({type: 'error', message: `파일 삭제 실패: ${error}`});
            }
        );
    }, [selectedMismatch, folderData, apiPrefix]);

    // ── 키 핸들러 ──

    const handleKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleAddKeyword();
        }
    }, [handleAddKeyword]);

    const handleRenameKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            handleRenameCategory();
        }
    }, [handleRenameCategory]);

    // ── 파생 값 ──

    const isSubcategory = selectedCategory.includes('/');
    const currentKeywords = selectedCategory ? (mappings[selectedCategory] || []) : [];

    const treeViewStyles = useMemo(() => ({
        height: 'fit-content',
        flexGrow: 1,
        overflowY: 'auto',
    }), []);

    // ── 렌더링 ──

    if (!isOpen) {
        return (
            <Card>
                <Card.Header
                    onClick={() => setIsOpen(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    {title}
                </Card.Header>
            </Card>
        );
    }

    return (
        <Card>
            <Card.Header
                onClick={() => setIsOpen(false)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={faChevronDown} className="me-2"/>
                {title}
            </Card.Header>
            <Card.Body>
                {message && (
                    <div className={`alert ${message.includes('실패') || message.includes('오류') ? 'alert-danger' : 'alert-info'} py-1 mb-2`}>
                        {message}
                    </div>
                )}
                {loading ? (
                    <div className="text-center p-4">
                        <Spinner animation="border"/>
                        <p className="mt-2">로딩 중...</p>
                    </div>
                ) : folderData.length === 0 ? (
                    <div className="text-muted p-3">카테고리 없음</div>
                ) : (
                    <Row className="g-0">
                        <Col md={4}>
                            <Card>
                                <Card.Header className="py-1">디렉토리 목록</Card.Header>
                                <div id="dir_list">
                                    <RichTreeView
                                        items={folderData}
                                        aria-label="category admin"
                                        sx={treeViewStyles}
                                        slots={{item: AdminTreeItem}}
                                        expandedItems={expandedItems}
                                        onSelectedItemsChange={handleTreeItemClick}
                                    />
                                </div>
                            </Card>
                        </Col>
                        <Col md={8}>
                            {/* 카테고리 선택 시 */}
                            {selectedCategory && !selectedMismatch && (
                                <Card>
                                    <Card.Header className="py-1 d-flex justify-content-between align-items-center">
                                        <span>
                                            <strong>{selectedCategory}</strong>
                                            {saving && <Spinner animation="border" size="sm" className="ms-2"/>}
                                        </span>
                                        <span>
                                            <Button
                                                variant="outline-secondary"
                                                size="sm"
                                                className="me-1"
                                                disabled={saving}
                                                onClick={() => {
                                                    setNewCategoryName(selectedCategory);
                                                    setShowRenameModal(true);
                                                }}
                                                title="이름 변경"
                                            >
                                                이름 변경 <FontAwesomeIcon icon={faEdit}/>
                                            </Button>
                                            <Button
                                                variant="outline-danger"
                                                size="sm"
                                                disabled={saving}
                                                onClick={() => setShowDeleteModal(true)}
                                                title="카테고리 삭제"
                                            >
                                                삭제 <FontAwesomeIcon icon={faTrash}/>
                                            </Button>
                                            <Button
                                                variant="outline-success"
                                                size="sm"
                                                className="ms-1"
                                                disabled={saving}
                                                onClick={() => setShowReloadModal(true)}
                                                title="ES 재적재"
                                            >
                                                {reloading ? <Spinner animation="border" size="sm"/> : <>ES 재적재 <FontAwesomeIcon icon={faRotate}/></>}
                                            </Button>
                                        </span>
                                    </Card.Header>
                                    <Card.Body>
                                        <Form.Check
                                            type="checkbox"
                                            id={`hidden-${contentType}-${selectedCategory}`}
                                            label="사용자 비노출"
                                            checked={hiddenCategories.has(selectedCategory)}
                                            onChange={() => handleToggleHidden(selectedCategory, hiddenCategories.has(selectedCategory))}
                                            disabled={saving}
                                            className="mb-2"
                                        />
                                        {!isSubcategory && (
                                            <>
                                                <InputGroup className="mb-2">
                                                    <Form.Control
                                                        ref={keywordInputRef}
                                                        type="text"
                                                        placeholder="새 키워드 입력"
                                                        value={newKeyword}
                                                        onChange={(e) => setNewKeyword(e.target.value)}
                                                        onKeyDown={handleKeyDown}
                                                        disabled={saving}
                                                    />
                                                    <Button
                                                        variant="outline-primary"
                                                        onClick={handleAddKeyword}
                                                        disabled={saving || !newKeyword.trim()}
                                                    >
                                                        <FontAwesomeIcon icon={faPlus}/> 추가
                                                    </Button>
                                                </InputGroup>
                                                <div className="d-flex flex-wrap gap-1">
                                                    {currentKeywords.map(keyword => (
                                                        <Badge
                                                            key={keyword}
                                                            bg="info"
                                                            className="d-flex align-items-center gap-1"
                                                            style={{fontSize: '0.85rem', padding: '0.4rem 0.6rem'}}
                                                        >
                                                            {keyword}
                                                            <FontAwesomeIcon
                                                                icon={faTrash}
                                                                style={{cursor: saving ? 'not-allowed' : 'pointer', marginLeft: '4px'}}
                                                                onClick={() => !saving && handleRemoveKeyword(keyword)}
                                                            />
                                                        </Badge>
                                                    ))}
                                                    {currentKeywords.length === 0 && (
                                                        <span className="text-muted">등록된 키워드가 없습니다.</span>
                                                    )}
                                                </div>
                                            </>
                                        )}
                                    </Card.Body>
                                </Card>
                            )}

                            {/* 불일치 항목 선택 시 */}
                            {selectedMismatch && (
                                <Card>
                                    <Card.Header className="py-1">
                                        <strong>{selectedMismatch.label}</strong>
                                    </Card.Header>
                                    <Card.Body>
                                        <div className="text-muted mb-2" style={{fontSize: '0.85rem'}}>
                                            {selectedMismatch.mismatchType === 'es_only'
                                                ? `${contentLabel} 정보만 존재하고 파일시스템에는 존재하지 않습니다.`
                                                : selectedMismatch.mismatchType === 'duplicate'
                                                    ? '동일한 파일 경로로 ES에 중복 문서가 존재합니다. 파일 삭제 후 재적재 시 발생할 수 있습니다.'
                                                    : `${contentLabel} 정보는 없고 파일시스템에만 존재합니다.`}
                                        </div>
                                        {selectedMismatch.mismatchType === 'duplicate' && selectedMismatch.dupDocs && (
                                            <table className="table table-sm table-bordered mb-2" style={{fontSize: '0.8rem'}}>
                                                <thead>
                                                    <tr>
                                                        <th>ID</th>
                                                        <th>제목</th>
                                                        <th>저자</th>
                                                        <th>액션</th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {selectedMismatch.dupDocs.map(doc => (
                                                        <tr key={doc.book_id}>
                                                            <td>{doc.book_id}</td>
                                                            <td>{doc.title}</td>
                                                            <td>{doc.author}</td>
                                                            <td>
                                                                <Button
                                                                    variant="outline-primary" size="sm" className="me-1 py-0"
                                                                    onClick={() => window.open(`/${contentType === 'comic' ? 'comics-view' : 'book-view'}/${doc.book_id}?category=${encodeURIComponent(selectedMismatch.category)}`, '_blank', 'noopener')}
                                                                >
                                                                    조회
                                                                </Button>
                                                                <Button
                                                                    variant="outline-danger" size="sm" className="py-0"
                                                                    onClick={() => {
                                                                        if (!window.confirm(`ID ${doc.book_id} (${doc.title}) 문서를 삭제하시겠습니까?`)) return;
                                                                        jsonDeleteReq(apiPrefix + '/books/' + doc.book_id, null,
                                                                            () => {
                                                                                setActionResult({type: 'success', message: `ID ${doc.book_id} 문서가 삭제되었습니다.`});
                                                                                setSelectedMismatch(null);
                                                                                loadData();
                                                                            },
                                                                            (error) => setActionResult({type: 'danger', message: error || '삭제 실패'})
                                                                        );
                                                                    }}
                                                                >
                                                                    삭제
                                                                </Button>
                                                            </td>
                                                        </tr>
                                                    ))}
                                                </tbody>
                                            </table>
                                        )}
                                        <div className="d-flex flex-wrap gap-1">
                                            {selectedMismatch.mismatchType === 'es_only' && (
                                                <>
                                                    <Button
                                                        variant="outline-warning" size="sm"
                                                        onClick={() => window.open(`/${contentType === 'comic' ? 'comics-edit' : 'book-edit'}/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`, '_blank', 'noopener')}
                                                    >
                                                        편집
                                                    </Button>
                                                    <Button
                                                        variant="outline-primary" size="sm"
                                                        onClick={() => window.open(`/${contentType === 'comic' ? 'comics-view' : 'book-view'}/${selectedMismatch.bookId}?category=${encodeURIComponent(selectedMismatch.category)}`, '_blank', 'noopener')}
                                                    >
                                                        조회
                                                    </Button>
                                                    <Button
                                                        variant="outline-danger" size="sm"
                                                        onClick={handleDeleteEsDoc}
                                                    >
                                                        삭제
                                                    </Button>
                                                </>
                                            )}
                                            {selectedMismatch.mismatchType === 'fs_only' && (
                                                <>
                                                    <Button
                                                        variant="outline-success" size="sm"
                                                        onClick={handleIndexFile}
                                                    >
                                                        ES 적재
                                                    </Button>
                                                    <Button
                                                        variant="outline-danger" size="sm"
                                                        onClick={handleDeleteFile}
                                                    >
                                                        파일 삭제
                                                    </Button>
                                                </>
                                            )}
                                        </div>
                                    </Card.Body>
                                </Card>
                            )}

                            {/* 아무것도 선택 안 됨 */}
                            {!selectedCategory && !selectedMismatch && !actionResult && (
                                <div className="text-muted p-3">왼쪽에서 디렉토리를 선택하세요.</div>
                            )}
                            {actionResult && (
                                <div className={`p-3 ${actionResult.type === 'success' ? 'text-success' : 'text-danger'}`} style={{fontSize: '0.85rem'}}>
                                    {actionResult.message}
                                </div>
                            )}
                        </Col>
                    </Row>
                )}
            </Card.Body>

            {/* 이름 변경 모달 */}
            <Modal show={showRenameModal} onHide={() => setShowRenameModal(false)} centered>
                <Modal.Header closeButton>
                    <Modal.Title>카테고리 이름 변경</Modal.Title>
                </Modal.Header>
                <Modal.Body>
                    <Form.Group>
                        <Form.Label>현재 이름</Form.Label>
                        <Form.Control type="text" value={selectedCategory} disabled />
                    </Form.Group>
                    <Form.Group className="mt-3">
                        <Form.Label>새 이름</Form.Label>
                        <Form.Control
                            type="text"
                            value={newCategoryName}
                            onChange={(e) => setNewCategoryName(e.target.value)}
                            onKeyDown={handleRenameKeyDown}
                            autoFocus
                        />
                    </Form.Group>
                </Modal.Body>
                <Modal.Footer>
                    <Button variant="secondary" onClick={() => setShowRenameModal(false)}>취소</Button>
                    <Button
                        variant="primary"
                        onClick={handleRenameCategory}
                        disabled={saving || !newCategoryName.trim() || newCategoryName.trim() === selectedCategory}
                    >
                        {saving ? <Spinner animation="border" size="sm" /> : '변경'}
                    </Button>
                </Modal.Footer>
            </Modal>

            {/* 삭제 확인 모달 */}
            <Modal show={showDeleteModal} onHide={() => setShowDeleteModal(false)} centered>
                <Modal.Header closeButton>
                    <Modal.Title>카테고리 삭제</Modal.Title>
                </Modal.Header>
                <Modal.Body>
                    <p className="text-danger fw-bold">
                        카테고리 &apos;{selectedCategory}&apos; 및 하위 카테고리의 모든 문서가 삭제됩니다.
                    </p>
                    <p className="text-muted">이 작업은 되돌릴 수 없습니다.</p>
                </Modal.Body>
                <Modal.Footer>
                    <Button variant="secondary" onClick={() => setShowDeleteModal(false)}>취소</Button>
                    <Button
                        variant="danger"
                        onClick={handleDeleteCategory}
                        disabled={saving}
                    >
                        {saving ? <Spinner animation="border" size="sm" /> : '삭제'}
                    </Button>
                </Modal.Footer>
            </Modal>

            {/* ES 재적재 확인 모달 */}
            <Modal show={showReloadModal} onHide={() => setShowReloadModal(false)} centered>
                <Modal.Header closeButton>
                    <Modal.Title>ES 재적재</Modal.Title>
                </Modal.Header>
                <Modal.Body>
                    <p className="fw-bold">
                        카테고리 &apos;{selectedCategory}&apos;의 모든 파일을 ES에 재적재합니다.
                    </p>
                    <p className="text-muted">하위 디렉토리를 포함하여 전체 재적재하며, 파일 수에 따라 수 분이 소요될 수 있습니다.</p>
                </Modal.Body>
                <Modal.Footer>
                    <Button variant="secondary" onClick={() => setShowReloadModal(false)}>취소</Button>
                    <Button
                        variant="success"
                        onClick={handleReloadCategory}
                        disabled={saving}
                    >
                        {reloading ? <Spinner animation="border" size="sm" /> : '재적재'}
                    </Button>
                </Modal.Footer>
            </Modal>
        </Card>
    );
}

CategoryAdmin.propTypes = {
    contentType: PropTypes.string,
    title: PropTypes.string,
};

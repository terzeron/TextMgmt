import './Folder.css';

import React, {useState, useMemo} from "react";
import PropTypes from 'prop-types';

import clsx from 'clsx';
import {animated, useSpring} from '@react-spring/web';
import {styled, alpha} from '@mui/material/styles';

import {Card} from "react-bootstrap";
import Box from '@mui/material/Box';
import Collapse from '@mui/material/Collapse';
import Typography from '@mui/material/Typography';
import ArticleIcon from '@mui/icons-material/Article';
import DeleteIcon from '@mui/icons-material/Delete';
import FolderOpenIcon from '@mui/icons-material/FolderOpen';
import FolderRounded from '@mui/icons-material/FolderRounded';
import ImageIcon from '@mui/icons-material/Image';
import PictureAsPdfIcon from '@mui/icons-material/PictureAsPdf';
import VideoCameraBackIcon from '@mui/icons-material/VideoCameraBack';
import {RichTreeView} from '@mui/x-tree-view/RichTreeView';
import {treeItemClasses} from '@mui/x-tree-view/TreeItem';
import {unstable_useTreeItem2 as useTreeItem2} from '@mui/x-tree-view/useTreeItem2';
import {TreeItem2Content, TreeItem2IconContainer, TreeItem2Label, TreeItem2Root} from '@mui/x-tree-view/TreeItem2';
import {TreeItem2Icon} from '@mui/x-tree-view/TreeItem2Icon';
import {TreeItem2Provider} from '@mui/x-tree-view/TreeItem2Provider';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import {faChevronDown, faChevronRight} from "@fortawesome/free-solid-svg-icons";
import {findFolderInTree} from './folderUtils';

function DotIcon() {
    return (
        <Box
            sx={{
                width: 6,
                height: 6,
                borderRadius: '70%',
                bgcolor: 'warning.main',
                display: 'inline-block',
                verticalAlign: 'middle',
                zIndex: 1,
                mx: 1,
            }}
        />
    );
}

const StyledTreeItemRoot = styled(TreeItem2Root)(({theme}) => ({
    color:
        theme.palette.mode === 'light'
            ? theme.palette.grey[800]
            : theme.palette.grey[400],
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
            color:
                theme.palette.mode === 'light'
                    ? theme.palette.primary.main
                    : theme.palette.primary.dark,
        },
        '&::before': {
            content: '""',
            display: 'block',
            position: 'absolute',
            left: '16px',
            top: '44px',
            height: 'calc(100% - 48px)',
            width: '1.5px',
            backgroundColor:
                theme.palette.mode === 'light'
                    ? theme.palette.grey[300]
                    : theme.palette.grey[700],
        },
    },
    '&:hover': {
        backgroundColor: alpha(theme.palette.primary.main, 0.1),
        color: theme.palette.mode === 'light' ? theme.palette.primary.main : 'white',
    },
    [`&.Mui-focused, &.Mui-selected, &.Mui-selected.Mui-focused`]: {
        backgroundColor:
            theme.palette.mode === 'light'
                ? theme.palette.primary.main
                : theme.palette.primary.dark,
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
    flex: 1,
    minWidth: 0,
});


const MemoizedIcon = React.memo(({ Icon, color }) => (
    <Box component={Icon} className="labelIcon" sx={{ mr: 1, fontSize: '1.2rem', color: color || 'inherit' }} />
));
MemoizedIcon.displayName = "MemoizedIcon";
MemoizedIcon.propTypes = {
    Icon: PropTypes.elementType.isRequired,
    color: PropTypes.string,
};

// eslint-disable-next-line react/prop-types
function CustomLabel({icon: Icon, iconColor, expandable, count, children, ...other}) {
    return (
        <TreeItem2Label
            {...other}
            sx={{
                display: 'flex',
                alignItems: 'center',
            }}
        >
            {Icon && <MemoizedIcon Icon={Icon} color={iconColor} />}
            <StyledTreeItemLabelText variant="body2">{children}</StyledTreeItemLabelText>
            {count > 0 && (
                <Typography variant="caption" sx={{ ml: 'auto', color: 'text.secondary', fontWeight: 400, transform: 'scale(0.6)', transformOrigin: 'right center', flexShrink: 0, whiteSpace: 'nowrap', width: '5em', textAlign: 'right' }}>
                    {count}
                </Typography>
            )}
            {expandable && <DotIcon/>}
        </TreeItem2Label>
    );
}

CustomLabel.propTypes = {
    icon: PropTypes.elementType,
    iconColor: PropTypes.string,
    expandable: PropTypes.bool,
    count: PropTypes.number,
    children: PropTypes.node,
};

const isExpandable = (reactChildren) => {
    if (Array.isArray(reactChildren)) {
        return reactChildren.length > 0 && reactChildren.some(isExpandable);
    }
    return Boolean(reactChildren);
};

const getIconFromFileType = (fileType) => {
    switch (fileType) {
        case 'image':
        case 'jpg':
        case 'jpeg':
        case 'png':
        case 'gif':
        case 'webp':
        case 'bmp':
        case 'tiff':
        case 'svg':
            return { icon: ImageIcon, color: '#4caf50' };  // 초록
        case 'pdf':
            return { icon: PictureAsPdfIcon, color: '#f44336' };  // 빨강
        case 'doc':
        case 'docx':
            return { icon: ArticleIcon, color: '#2196f3' };  // 파랑
        case 'epub':
            return { icon: ArticleIcon, color: '#9c27b0' };  // 보라
        case 'rtf':
        case 'html':
        case 'txt':
            return { icon: ArticleIcon, color: '#607d8b' };  // 회색
        case 'video':
            return { icon: VideoCameraBackIcon, color: '#ff9800' };  // 주황
        case 'folder':
            return { icon: FolderRounded, color: '#ffc107' };  // 노랑
        case 'pinned':
            return { icon: FolderOpenIcon, color: '#ffc107' };
        case 'trash':
            return { icon: DeleteIcon, color: '#9e9e9e' };
        default:
            return { icon: ArticleIcon, color: '#607d8b' };
    }
};

const CustomTreeItem = React.forwardRef(function CustomTreeItem(props, ref) {
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
    const { icon, iconColor } = useMemo(() => {
        if (expandable || item?.fileType === 'folder') return { icon: FolderRounded, iconColor: '#ffc107' };
        const result = getIconFromFileType(item?.fileType);
        return { icon: result.icon, iconColor: result.color };
    }, [expandable, item?.fileType]);

    return (
        <TreeItem2Provider itemId={itemId}>
            <StyledTreeItemRoot {...getRootProps(other)}>
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
                        <TreeItem2Icon status={status}/>
                    </TreeItem2IconContainer>

                    <CustomLabel
                        {...getLabelProps({icon, iconColor, expandable: expandable && status.expanded, count: item?.count})}
                    />
                </CustomTreeItemContent>
                {children && <TransitionComponent {...getGroupTransitionProps()} />}
            </StyledTreeItemRoot>
        </TreeItem2Provider>
    );
});

export default function Folder(props) {
    const [expandedItems, setExpandedItems] = useState([]);
    const defaultExpandedItems = useMemo(() => props.folderData.map(o => o.id), [props.folderData]);
    const treeViewStyles = useMemo(() => ({
        height: 'fit-content',
        flexGrow: 1,
        maxWidth: 600,
        overflowY: 'auto',
    }), []);

    if (!props.isOpen) {
        return (
            <Card>
                <Card.Header
                    onClick={() => props.onToggle(true)}
                    style={{cursor: 'pointer', userSelect: 'none'}}
                    className="py-2">
                    <FontAwesomeIcon icon={faChevronRight} className="me-2"/>
                    디렉토리
                </Card.Header>
            </Card>
        );
    }

    return (
        <Card>
            <Card.Header
                onClick={() => props.onToggle(false)}
                style={{cursor: 'pointer', userSelect: 'none'}}
                className="py-2">
                <FontAwesomeIcon icon={faChevronDown} className="me-2"/>
                디렉토리
            </Card.Header>
            <Card.Body>
                <div id="dir_list">
                    {props.folderData && (
                        <RichTreeView
                            items={props.folderData}
                            aria-label="file explorer"
                            sx={treeViewStyles}
                            slots={{item: CustomTreeItem}}
                            defaultExpandedItems={defaultExpandedItems}
                            expandedItems={expandedItems}
                            selectedItems={props.selectedItems}
                            onSelectedItemsChange={(event, selectedId) => {
                                const found = findFolderInTree(props.folderData, selectedId);
                                const isFolder = found && found.fileType === 'folder';
                                setExpandedItems((prevExpandedItems) => {
                                    if (prevExpandedItems.includes(selectedId)) {
                                        return prevExpandedItems.filter(x => x !== selectedId);
                                    } else {
                                        return [...prevExpandedItems, selectedId];
                                    }
                                });

                                props.onClickHandler(selectedId);
                                if (!isFolder) {
                                    props.onToggle(false);
                                }
                            }}
                        />
                    )}
                </div>
            </Card.Body>
        </Card>
    );
}

Folder.propTypes = {
    folderData: PropTypes.array.isRequired,
    selectedItems: PropTypes.array,
    onClickHandler: PropTypes.func.isRequired,
    isOpen: PropTypes.bool.isRequired,
    onToggle: PropTypes.func.isRequired,
}
import './Folder.css';

import React, {useState} from "react";
import PropTypes from 'prop-types';

import clsx from 'clsx';
import {animated, useSpring} from '@react-spring/web';
import {styled, alpha} from '@mui/material/styles';

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
});


// eslint-disable-next-line react/prop-types
function CustomLabel({icon: Icon, expandable, children, ...other}) {
    return (
        <TreeItem2Label
            {...other}
            sx={{
                display: 'flex',
                alignItems: 'center',
            }}
        >
            {Icon && (
                <Box
                    component={Icon}
                    className="labelIcon"
                    color="inherit"
                    sx={{mr: 1, fontSize: '1.2rem'}}
                />
            )}

            <StyledTreeItemLabelText variant="body2">{children}</StyledTreeItemLabelText>
            {expandable && <DotIcon/>}
        </TreeItem2Label>
    );
}

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
            return ImageIcon;
        case 'pdf':
            return PictureAsPdfIcon;
        case 'doc':
        case 'epub':
        case 'rtf':
        case 'html':
        case 'txt':
            return ArticleIcon;
        case 'video':
            return VideoCameraBackIcon;
        case 'folder':
            return FolderRounded;
        case 'pinned':
            return FolderOpenIcon;
        case 'trash':
            return DeleteIcon;
        default:
            return ArticleIcon;
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

    const item = publicAPI.getItem(itemId);
    const expandable = isExpandable(children);
    let icon;
    if (expandable) {
        icon = FolderRounded;
    } else if (item.fileType) {
        icon = getIconFromFileType(item.fileType);
    }

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
                        {...getLabelProps({icon, expandable: expandable && status.expanded})}
                    />
                </CustomTreeItemContent>
                {children && <TransitionComponent {...getGroupTransitionProps()} />}
            </StyledTreeItemRoot>
        </TreeItem2Provider>
    );
});

export default function Folder(props) {
    const [expandedItems, setExpandedItems] = useState([]);

    return (
        <div id="dir_list">
            {props.folderData && (
                <RichTreeView
                    items={props.folderData}
                    aria-label="file explorer"
                    sx={{height: 'fit-content', flexGrow: 1, maxWidth: 400, overflowY: 'auto'}}
                    slots={{item: CustomTreeItem}}
                    defaultExpandedItems={props.folderData.map(o => o.id)}
                    expandedItems={expandedItems}
                    selectedItems={props.selectedItems}
                    onSelectedItemsChange={(event, selectedId) => {
                        // in case of category entry, expand the selected category
                        if (props.folderData.find(o => o.id === selectedId)) {
                            const expanded = expandedItems.includes(selectedId)
                                ? expandedItems.filter(x => x !== selectedId)
                                : [...expandedItems, selectedId];
                            setExpandedItems(expanded);
                        }
                        props.onClickHandler(selectedId);
                    }}
                />
            )
            }
        </div>
    );
}

Folder.propTypes = {
    folderData: PropTypes.array.isRequired,
    selectedItems: PropTypes.array,
    onClickHandler: PropTypes.func.isRequired,
}
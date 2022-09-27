import {Provider, teamsTheme, Tree} from '@fluentui/react-northstar'
import {faFolderOpen, faFolderClosed} from '@fortawesome/free-solid-svg-icons';
import {FontAwesomeIcon} from "@fortawesome/react-fontawesome";
import './DirList.css';


export default function DirList(props) {
    const titleRenderer = (Component, {content, expanded, open, hasSubtree, ...restProps}) => (
        <Component expanded={expanded} hasSubtree={hasSubtree} {...restProps}>
            {hasSubtree ? (expanded ? <FontAwesomeIcon icon={faFolderOpen}/> : <FontAwesomeIcon icon={faFolderClosed}/>) : ""}
            {content}
        </Component>
    );

    return (
        <Provider theme={teamsTheme} id="dir_list">
            <Tree
                items={props.data}
                renderItemTitle={titleRenderer}
                aria-label="Initially open"
                defaultActiveItemIds={[
                    'root',
                ]}
                className="ps-0 pe-0"
            />
        </Provider>
    );
}


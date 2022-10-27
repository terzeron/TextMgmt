import './DirList.css';

import TreeMenu from 'react-simple-tree-menu';
import 'react-simple-tree-menu/dist/main.css';

export default function DirList({treeData, onClickHandler}) {
  return (
    <TreeMenu data={treeData} debounceTime={125} onClickItem={({key, ...props}) => {
      if (props["hasNodes"] === false) {
        onClickHandler(key);
      }
    }}>
    </TreeMenu>
  );
}

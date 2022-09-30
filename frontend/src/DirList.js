import './DirList.css';

import {useEffect, useState} from 'react';

import TreeMenu from 'react-simple-tree-menu';
import 'react-simple-tree-menu/dist/main.css';
import {getUrlPrefix, handleFetchErrors} from './Common';

export default function DirList({onClickHandler}) {
  const [treeData, setTreeData] = useState([]);

  useEffect(() => {
    fetch(`${getUrlPrefix()}/dirs`)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result['status'] === 'success') {
              setTreeData(result['result']);
            } else {
              console.error(`dir list load failed, ${result['error']}`);
            }
          })
          .catch((error) => {
            console.error(`dir list load failed, ${error}`);
          });
      });

    return () => {
      // console.log('컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함');
    };
  }, [] /* rendered once */);

  return (
    <TreeMenu data={treeData} debounceTime={125} onClickItem={({key, label, ...props}) => { onClickHandler(key, label, props); } }>
    </TreeMenu>
  );
}

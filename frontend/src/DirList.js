import './DirList.css';

import {useCallback, useEffect, useState} from 'react';

import TreeMenu from 'react-simple-tree-menu';
import 'react-simple-tree-menu/dist/main.css';
import {getUrlPrefix, handleFetchErrors} from './Common';

export default function DirList({onClickHandler}) {
  const [treeData, setTreeData] = useState([]);
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState('');

  useEffect(() => {
    console.log("call /somedirs for treeData");

    setLoading(true);
    fetch(`${getUrlPrefix()}/somedirs`)
      .then(handleFetchErrors)
      .then((response) => {
        response.json()
          .then((result) => {
            if (result['status'] === 'success') {
              setTreeData(result['result']);

              console.log("call /dirs for treeData");
              fetch(`${getUrlPrefix()}/dirs`)
                .then(handleFetchErrors)
                .then((response) => {
                  response.json()
                    .then((result) => {
                      if (result['status'] === 'success') {
                        setTreeData(result['result']);
                      } else {
                        setErrorMessage(`directory list load failed, ${result['error']}`);
                      }
                    })
                    .catch((error) => {
                      setErrorMessage(`dir list load failed, ${error}`);
                    });
                })
                .catch((error) => {
                  setErrorMessage(`dir list load failed, ${error}`);
                });
            } else {
              setErrorMessage(`directory list load failed, ${result['error']}`);
            }
          })
          .catch((error) => {
            setErrorMessage(`dir list load failed, ${error}`);
          })
          .finally(() => {
            setLoading(false);
          });
      });

    return () => {
      // console.log('컴퍼넌트가 사라질 때 cleanup할 일을 여기서 처리해야 함');
    };
  }, [] /* rendered once */);

  if (loading) return <div>로딩 중...</div>;
  if (errorMessage) return <div>{errorMessage}</div>;
  return (
    <TreeMenu data={treeData} debounceTime={125} onClickItem={({key, label, ...props}) => {
      if (props['level'] > 0) {
        const next = treeData[props['level']][props['index'] + 1];
        const subTreeData = treeData[props['index']].nodes;
        let nextIndex = 0;
        for (let i = 0; i < subTreeData.length; i++) {
          if (subTreeData[i]['key'] === label) {
            nextIndex = i + 1;
            break;
          }
        }
        onClickHandler(key, label, props, subTreeData, nextIndex);
      }
    }}>
    </TreeMenu>
  );
}

import {create} from 'zustand';
import {jsonGetReq} from "../Common";

const useSearchStore = create((set) => ({
    query: '',
    searchResult: [],
    isLoading: false,
    error: null,
    setQuery: (query) => set({query}),
    search: (query) => {
        console.log(`[searchStore] search called with query: '${query}'`);
        set({isLoading: true, error: null});
        try {
            if (!query || query.trim() === '') {
                console.log('[searchStore] query is empty, skipping search.');
                set({searchResult: [], isLoading: false});
                return;
            }
            const searchUrl = `/search/${encodeURIComponent(query)}`;
            console.log(`[searchStore] requesting url: ${searchUrl}`);
            jsonGetReq(searchUrl, null, (data) => {
                console.log('[searchStore] search success:', data);
                set({searchResult: data, isLoading: false});
            }, (err) => {
                console.error('[searchStore] search error:', err);
                set({error: err, isLoading: false});
            });
        } catch (err) {
            console.error('[searchStore] search caught exception:', err);
            set({error: err, isLoading: false});
        }
    },
}));

export default useSearchStore; 
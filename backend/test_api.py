#!/usr/bin/env python

import requests
import random
from collections import Counter
import re
from konlpy.tag import Okt

# 기본 API 엔드포인트
BASE_URL = "http://127.0.0.1:8000"

def get_top_frequent_words(text, top_n=100):
    """요약 텍스트에서 명사를 추출하고 가장 빈번하게 나오는 단어 목록을 찾습니다."""
    if not text:
        return []
    
    okt = Okt()
    
    # 명사만 추출
    nouns = okt.nouns(text)
    
    # 두 글자 이상인 명사만 선택
    meaningful_words = [word for word in nouns if len(word) > 1]
    
    if not meaningful_words:
        return []
        
    word_counts = Counter(meaningful_words)
    return word_counts.most_common(top_n)

def run_api_exploration_test():
    """API를 점진적으로 탐색하며 테스트 시나리오를 실행합니다."""
    
    print("🚀 API 탐색 테스트를 시작합니다.")
    
    # 1. 책이 있는 카테고리 찾기
    print("\n[1단계] 가장 책이 많은 카테고리 검색 중...")
    try:
        response = requests.get(f"{BASE_URL}/categories")
        response.raise_for_status()
        categories = response.json().get("result", [])
        if not categories:
            print("❌ 실패: 카테고리를 찾을 수 없습니다.")
            return
    except requests.RequestException as e:
        print(f"❌ 실패: 카테고리 목록 조회 중 오류 발생: {e}")
        return

    selected_category = None
    category_books = []
    max_book_count = 0

    for category in categories:
        try:
            #print(f"  - 카테고리 '{category}' 확인 중...")
            cat_response = requests.get(f"{BASE_URL}/categories/{category}")
            cat_response.raise_for_status()
            books = cat_response.json().get("result", [])
            if len(books) > max_book_count:
                max_book_count = len(books)
                selected_category = category
                category_books = books
        except requests.RequestException:
            continue
    
    if selected_category:
        print(f"✅ 성공: 가장 책이 많은 '{selected_category}' 카테고리에서 책 {len(category_books)}권을 찾았습니다.")
    else:
        print("❌ 실패: 책이 있는 카테고리를 찾지 못했습니다.")
        return

    # 2 & 3: 책 ID 확보 및 유사 책 검색
    print("\n[2단계 & 3단계] 유사한 책이 있는 책 ID를 찾습니다...")
    similar_books = []
    selected_book_id = None

    # 책 목록을 무작위로 섞어 다양한 책을 시도
    # random.shuffle(category_books) 

    for book in category_books:
        book_id = book.get("book_id")
        file_type = book.get("file_type", "")
        if not book_id or file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
            continue
        
        print(f"\n  - [2단계] 책 ID '{book_id}'(제목: {book.get('title')}, 타입: {file_type}) 선택.")
        print(f"  - [3단계] 해당 ID로 유사한 책 검색 시도...")
        try:
            similar_response = requests.get(f"{BASE_URL}/similar/{book_id}")
            similar_response.raise_for_status()
            
            result = similar_response.json().get("result", [])
            if result:
                selected_book_id = book_id
                similar_books = result
                print(f"✅ [3단계] 성공: 책 ID '{selected_book_id}'에서 유사한 책 {len(similar_books)}권을 찾았습니다.")
                print(f"  - 첫 번째 유사 책: {similar_books[0].get('title')}")
                break
            else:
                print(f"  - [3단계] 정보: 유사한 책이 없습니다. 다른 책으로 재시도합니다.")
        except requests.RequestException as e:
            print(f"  - [3단계] 오류: {e}. 다른 책으로 재시도합니다.")
            continue
            
    if not selected_book_id:
        print("❌ 실패: 유사한 책을 가진 책을 찾지 못했습니다.")
        return

    # 4. 키워드 검색 수행
    print("\n[4단계] 유사한 책 제목과 요약에서 키워드를 추출하여 검색 수행 중...")
    
    # 모든 유사 책의 제목과 요약을 합칩니다.
    combined_text = " ".join(
        f"{book.get('title', '')} {book.get('summary', '')}".strip() 
        for book in similar_books
    )
    print(f"  - 수집된 전체 텍스트의 길이: {len(combined_text)} 자")
    if len(combined_text) < 200:
        print(f"  - 수집된 텍스트 (일부): \"{combined_text[:200]}...\"")

    top_keywords = get_top_frequent_words(combined_text, 10)
    
    if not top_keywords:
        print("❌ 실패: 유사한 책들의 제목과 요약에서 유효한 키워드를 찾지 못했습니다.")
        return
        
    print("  - 유사 책 제목과 요약에서 찾은 명사 (빈도순):")
    for i, (word, count) in enumerate(top_keywords):
        print(f"    {i+1:2d}. {word} ({count}회)")

    # 가장 빈도가 높은 단어를 검색에 사용
    keyword = top_keywords[0][0]
    print(f"\n  - 검색에 사용할 키워드: '{keyword}'")

    try:
        search_response = requests.get(f"{BASE_URL}/search/{keyword}")
        search_response.raise_for_status()
        search_results = search_response.json().get("result", [])

        if search_results:
            print(f"✅ 성공: 키워드 '{keyword}'로 검색하여 {len(search_results)}개의 결과를 얻었습니다.")
            print(f"  - 첫 번째 결과: {search_results[0].get('title')}")
        else:
            print(f"❌ 실패: 키워드 '{keyword}'에 대한 검색 결과가 없습니다.")
            return
            
    except requests.RequestException as e:
        print(f"❌ 실패: 키워드 검색 중 오류 발생: {e}")
        return

    print("\n🎉 모든 테스트 시나리오를 성공적으로 완료했습니다.")


if __name__ == "__main__":
    run_api_exploration_test() 
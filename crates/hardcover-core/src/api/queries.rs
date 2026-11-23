//! Every GraphQL document the client sends.
//!
//! Each document has exactly one top-level field: Hardcover charges one rate-limit
//! token per top-level field and rejects requests with more fields than the burst.
//! The test at the bottom validates every document against `schema/hardcover.graphql`.

macro_rules! book_summary_fields {
    () => {
        "fragment BookSummaryFields on books {
  id title subtitle slug pages release_year rating users_count cached_image cached_contributors
}
"
    };
}

macro_rules! user_book_fields {
    () => {
        "fragment UserBookFields on user_books {
  id user_id status_id rating date_added updated_at privacy_setting_id private_notes
  book {
    ...BookSummaryFields
    default_audio_edition_id
    book_series(order_by: [{featured: desc}, {position: asc_nulls_last}]) {
      position featured compilation series { id name slug }
    }
  }
  edition { ...EditionFields }
  user_book_reads(order_by: {id: desc}) {
    id started_at finished_at progress progress_pages progress_seconds edition_id
    edition { id title pages audio_seconds edition_format reading_format_id }
  }
}
"
    };
}

/// The edition picker's fields, also used for an entry's own edition.
macro_rules! edition_fields {
    () => {
        "fragment EditionFields on editions {
  id title pages audio_seconds edition_format release_date isbn_13 cached_image
  reading_format_id reading_format { format } publisher { name } language { language }
}
"
    };
}

/// The user's review of an entry. Not part of `UserBookFields`: review text can
/// be long, and only the book page shows it.
macro_rules! review_fields {
    () => {
        "fragment ReviewFields on user_books {
  review_markdown review_raw review_has_spoilers reviewed_at has_review
}
"
    };
}

macro_rules! list_fields {
    () => {
        "fragment ListFields on lists {
  id name slug description privacy_setting_id ranked books_count default_view featured_profile url
}
"
    };
}

macro_rules! goal_fields {
    () => {
        "fragment GoalFields on goals {
  id metric goal progress description start_date end_date conditions archived completed_at
  privacy_setting_id
}
"
    };
}

pub(crate) const ME: &str = "query Me {
  me { id username name image { url width height } }
}
";

/// Keyset pagination over one user's shelves, Ignored (6) included.
pub(crate) const LIBRARY_PAGE: &str = concat!(
    "query LibraryPage($userId: Int!, $afterId: Int!, $limit: Int!) {
  user_books(
    where: {user_id: {_eq: $userId}, id: {_gt: $afterId}, status_id: {_in: [1, 2, 3, 4, 5, 6]}}
    order_by: {id: asc}
    limit: $limit
  ) { ...UserBookFields }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

/// Not scoped to a user: used to re-read an entry right after mutating it.
pub(crate) const USER_BOOK_BY_ID: &str = concat!(
    "query UserBookById($id: Int!) {
  user_books_by_pk(id: $id) { ...UserBookFields }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

pub(crate) const USER_BOOK_FOR_USER: &str = concat!(
    "query UserBookForUser($userId: Int!, $id: Int!) {
  user_books(where: {id: {_eq: $id}, user_id: {_eq: $userId}}, limit: 1) { ...UserBookFields }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

pub(crate) const USER_BOOK_FOR_BOOK: &str = concat!(
    "query UserBookForBook($userId: Int!, $bookId: Int!) {
  user_books(where: {user_id: {_eq: $userId}, book_id: {_eq: $bookId}}, order_by: {id: asc}, limit: 1) {
    ...UserBookFields
  }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

pub(crate) const SEARCH_BOOKS: &str =
    "query SearchBooks($query: String!, $page: Int!, $perPage: Int!) {
  search(query: $query, query_type: \"book\", page: $page, per_page: $perPage) {
    error page per_page results
  }
}
";

/// Author search. `ids` lists the hits' ids in order, for documents without one.
pub(crate) const SEARCH_AUTHORS: &str =
    "query SearchAuthors($query: String!, $page: Int!, $perPage: Int!) {
  search(query: $query, query_type: \"author\", page: $page, per_page: $perPage) {
    error page per_page ids results
  }
}
";

/// Series search. `ids` lists the hits' ids in order, for documents without one.
pub(crate) const SEARCH_SERIES: &str =
    "query SearchSeries($query: String!, $page: Int!, $perPage: Int!) {
  search(query: $query, query_type: \"series\", page: $page, per_page: $perPage) {
    error page per_page ids results
  }
}
";

/// Ids of the books the most readers finished in `$duration`, in ranked order.
pub(crate) const BOOKS_TRENDING: &str =
    "query BooksTrending($duration: TrendingDuration!, $limit: Int!) {
  books_trending(duration: $duration, limit: $limit, offset: 0) { error ids }
}
";

/// A book page's facts, with the exact votes for the tags in `$tags` (the
/// moods, paces and character tags of [`crate::tags::VOTE_TAG_IDS`]): the
/// exact counts behind the mood line; `cached_tags` keeps only the top few. `cached_tags` stays for the genres, warnings and tags.
pub(crate) const BOOK_DETAILS: &str = concat!(
    "query BookDetails($id: Int!, $tags: [Int!]!) {
  books_by_pk(id: $id) {
    ...BookSummaryFields
    description release_date editions_count ratings_count cached_tags
    ratings_distribution alternative_titles cached_similar_book_ids
    book_series(order_by: [{featured: desc}, {position: asc_nulls_last}]) {
      position featured series { id name slug }
    }
    taggable_counts(where: {tag_id: {_in: $tags}}) { tag_id count }
  }
}
",
    book_summary_fields!()
);

/// A tag's books, the most voted for it first (`count`; never `users_count`,
/// which puts the same blockbusters first whatever the tag). Merged
/// duplicates are filtered here. Paged by offset; the book id breaks ties, so
/// pages over equal counts neither repeat nor skip a book (the ordering by
/// `count` was verified live; the tiebreak was not).
pub(crate) const BOOKS_BY_TAG: &str = concat!(
    "query BooksByTag($tag: Int!, $limit: Int!, $offset: Int!) {
  taggable_counts(
    where: {
      tag_id: {_eq: $tag}
      taggable_type: {_eq: \"Book\"}
      book: {canonical_id: {_is_null: true}}
    }
    order_by: [{count: desc}, {taggable_id: asc}]
    limit: $limit
    offset: $offset
  ) {
    count
    book { ...BookSummaryFields }
  }
}
",
    book_summary_fields!()
);

/// [`BOOKS_BY_TAG`] narrowed to the books with at least `$min` votes for a
/// second tag `$other` (a genre within a mood, a mood within a genre). A
/// separate document rather than a `where` passed whole as a variable, which
/// would hide the filtered fields from the scope check.
pub(crate) const BOOKS_BY_TAG_WITH: &str = concat!(
    "query BooksByTagWith($tag: Int!, $other: Int!, $min: Int!, $limit: Int!, $offset: Int!) {
  taggable_counts(
    where: {
      tag_id: {_eq: $tag}
      taggable_type: {_eq: \"Book\"}
      book: {
        canonical_id: {_is_null: true}
        taggable_counts: {tag_id: {_eq: $other}, count: {_gte: $min}}
      }
    }
    order_by: [{count: desc}, {taggable_id: asc}]
    limit: $limit
    offset: $offset
  ) {
    count
    book { ...BookSummaryFields }
  }
}
",
    book_summary_fields!()
);

/// A batch of books' genres (`cached_tags`' Genre list) and their votes for
/// the tags in `$tags` (the 14 moods): what a shelf's genre and mood filter
/// needs.
pub(crate) const BOOK_TAGS: &str = "query BookTags($ids: [Int!]!, $limit: Int!, $tags: [Int!]!) {
  books(where: {id: {_in: $ids}}, limit: $limit) {
    id
    cached_tags(path: \"Genre\")
    taggable_counts(where: {tag_id: {_in: $tags}}) { tag_id count }
  }
}
";

/// A series and its books. Merged duplicates are filtered here; the client keeps
/// the most-read book per position and drops partial books and compilations,
/// as Hardcover's "Getting All Books in a Series" guide describes.
pub(crate) const SERIES_PAGE: &str = concat!(
    "query SeriesPage($id: Int!, $limit: Int!) {
  series_by_pk(id: $id) {
    id name slug description primary_books_count is_completed
    author { id name }
    book_series(
      where: {book: {canonical_id: {_is_null: true}}}
      order_by: [{position: asc_nulls_last}, {book: {users_count: desc}}]
      limit: $limit
    ) {
      position compilation
      book { ...BookSummaryFields is_partial_book }
    }
  }
}
",
    book_summary_fields!()
);

/// An author and their most-read books, filtered as in Hardcover's guide to an
/// author's books, with each book's series (merged series filtered here and
/// again by the client), from which the client works out the author's series.
/// Partial books are dropped by the client.
pub(crate) const AUTHOR_PAGE: &str = concat!(
    "query AuthorPage($id: Int!, $limit: Int!) {
  authors_by_pk(id: $id) {
    id name slug bio cached_image born_year death_year books_count
    contributions(
      where: {contributable_type: {_eq: \"Book\"}, book: {canonical_id: {_is_null: true}}}
      order_by: {book: {users_count: desc}}
      limit: $limit
    ) {
      contribution
      book {
        ...BookSummaryFields is_partial_book
        book_series(where: {series: {canonical_id: {_is_null: true}}}) {
          series { id name author_id canonical_id primary_books_count is_completed }
        }
      }
    }
  }
}
",
    book_summary_fields!()
);

/// A few books by id, e.g. a book's similar books.
pub(crate) const BOOKS_BY_IDS: &str = concat!(
    "query BooksByIds($ids: [Int!]!, $limit: Int!) {
  books(where: {id: {_in: $ids}}, limit: $limit) { ...BookSummaryFields }
}
",
    book_summary_fields!()
);

/// Trending books by id, each with its series (featured first), so Discover
/// can show one book per series. `BOOKS_BY_IDS` plus `book_series`, as
/// `BOOK_DETAILS` asks for it; only trending's second request uses it.
pub(crate) const TRENDING_BOOKS: &str = concat!(
    "query TrendingBooks($ids: [Int!]!, $limit: Int!) {
  books(where: {id: {_in: $ids}}, limit: $limit) {
    ...BookSummaryFields
    book_series(order_by: [{featured: desc}, {position: asc_nulls_last}]) {
      position featured series { id name slug }
    }
  }
}
",
    book_summary_fields!()
);

/// Books in these series released after `$after`, the soonest first. Merged
/// duplicates are filtered here; the client merges a book listed under two
/// series and drops partial books and compilations.
pub(crate) const UPCOMING_IN_SERIES: &str = concat!(
    "query UpcomingInSeries($series: [Int!]!, $after: date!, $limit: Int!) {
  book_series(
    where: {
      series_id: {_in: $series}
      book: {release_date: {_gt: $after}, canonical_id: {_is_null: true}}
    }
    order_by: [{book: {release_date: asc}}, {book_id: asc}]
    limit: $limit
  ) {
    position compilation
    series { id name slug }
    book { ...BookSummaryFields release_date is_partial_book }
  }
}
",
    book_summary_fields!()
);

/// The series a reader has started, a batch at a time, with every book of each:
/// the Series destination's data. Merged duplicates, compilations and books
/// without a position are filtered here; several books may share a position
/// (unmerged duplicates, translations), and the app keeps the one in the library,
/// else the most-read one, so no row is dropped by position here. Ordered by
/// position, so the row limit only ever cuts a series' tail.
pub(crate) const SERIES_OVERVIEW: &str = concat!(
    "query SeriesOverview($ids: [Int!]!, $limit: Int!, $rows: Int!) {
  series(where: {id: {_in: $ids}}, limit: $limit) {
    id name slug primary_books_count is_completed
    author { id name }
    book_series(
      where: {
        book: {canonical_id: {_is_null: true}}
        compilation: {_eq: false}
        position: {_is_null: false}
      }
      order_by: [{position: asc}, {book: {users_count: desc}}]
      limit: $rows
    ) {
      position compilation
      book { ...BookSummaryFields canonical_id release_date is_partial_book }
    }
  }
}
",
    book_summary_fields!()
);

/// Lengths needed to turn a percentage into pages/seconds, plus the read to update.
/// `$reads` selects either one read by id or the unfinished read.
pub(crate) const PROGRESS_CONTEXT: &str =
    "query ProgressContext($id: Int!, $reads: user_book_reads_bool_exp!) {
  user_books(where: {id: {_eq: $id}}, limit: 1) {
    id status_id
    book { pages audio_seconds }
    edition { pages audio_seconds reading_format_id }
    user_book_reads(where: $reads, order_by: {id: desc}) {
      id started_at finished_at edition { pages audio_seconds reading_format_id }
    }
  }
}
";

pub(crate) const INSERT_USER_BOOK: &str = concat!(
    "mutation InsertUserBook($object: UserBookCreateInput!) {
  insert_user_book(object: $object) { error id user_book { ...UserBookFields } }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

pub(crate) const UPDATE_USER_BOOK: &str = concat!(
    "mutation UpdateUserBook($id: Int!, $object: UserBookUpdateInput!) {
  update_user_book(id: $id, object: $object) { error id user_book { ...UserBookFields } }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

pub(crate) const DELETE_USER_BOOK: &str = "mutation DeleteUserBook($id: Int!) {
  delete_user_book(id: $id) { id book_id user_id }
}
";

/// Returns the written read and, nested, the whole entry as of this write:
/// reading the entry back in a separate request right afterwards can be stale.
pub(crate) const INSERT_USER_BOOK_READ: &str = concat!(
    "mutation InsertUserBookRead($userBookId: Int!, $read: DatesReadInput!) {
  insert_user_book_read(user_book_id: $userBookId, user_book_read: $read) {
    error id
    user_book_read {
      id user_book_id started_at finished_at progress progress_pages progress_seconds edition_id
      edition { id title pages audio_seconds edition_format }
      user_book { ...UserBookFields }
    }
  }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

/// Returns the written read and, nested, the whole entry as of this write:
/// reading the entry back in a separate request right afterwards can be stale.
pub(crate) const UPDATE_USER_BOOK_READ: &str = concat!(
    "mutation UpdateUserBookRead($id: Int!, $read: DatesReadInput!) {
  update_user_book_read(id: $id, object: $read) {
    error id
    user_book_read {
      id user_book_id started_at finished_at progress progress_pages progress_seconds edition_id
      edition { id title pages audio_seconds edition_format }
      user_book { ...UserBookFields }
    }
  }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!()
);

/// Deletes a read. Whether its journal entries go too is unverified. Asks only for the id:
/// relations of a just-deleted row may not resolve, and an error there would
/// hide a delete that happened. The client reads the entry back instead.
pub(crate) const DELETE_USER_BOOK_READ: &str = "mutation DeleteUserBookRead($id: Int!) {
  delete_user_book_read(id: $id) { error id }
}
";

/// A book's editions for the edition picker, most read first.
pub(crate) const BOOK_EDITIONS: &str = concat!(
    "query BookEditions($bookId: Int!, $limit: Int!, $offset: Int!) {
  editions(
    where: {book_id: {_eq: $bookId}}
    order_by: [{users_count: desc}, {id: asc}]
    limit: $limit
    offset: $offset
  ) { ...EditionFields }
}
",
    edition_fields!()
);

/// Puts an edition on the user's Owned list (created on first use). Only adds:
/// taking it off is `DELETE_LIST_BOOK`. The row tells the client the Owned
/// list's id, which it may not have known.
pub(crate) const EDITION_OWNED: &str = "mutation EditionOwned($id: Int!) {
  edition_owned(id: $id) { id list_book { id list_id book_id edition_id } }
}
";

/// Keyset pagination over one user's lists. Hardcover returns at most 100 rows.
pub(crate) const LISTS: &str = concat!(
    "query Lists($userId: Int!, $afterId: Int!, $limit: Int!) {
  lists(where: {user_id: {_eq: $userId}, id: {_gt: $afterId}}, order_by: {id: asc}, limit: $limit) {
    ...ListFields
  }
}
",
    list_fields!()
);

/// Re-reads a list when a mutation's payload left it out.
pub(crate) const LIST_BY_ID: &str = concat!(
    "query ListById($id: Int!) {
  lists_by_pk(id: $id) { ...ListFields }
}
",
    list_fields!()
);

/// Keyset pagination over every book on one user's lists, without the books.
pub(crate) const LIST_MEMBERSHIPS: &str =
    "query ListMemberships($userId: Int!, $afterId: Int!, $limit: Int!) {
  list_books(
    where: {list: {user_id: {_eq: $userId}}, id: {_gt: $afterId}}
    order_by: {id: asc}
    limit: $limit
  ) { id list_id book_id edition_id }
}
";

/// The user's lists one book is on: for when [`LIST_MEMBERSHIPS`] was cut short.
pub(crate) const BOOK_LIST_MEMBERSHIPS: &str =
    "query BookListMemberships($userId: Int!, $bookId: Int!, $limit: Int!) {
  list_books(
    where: {book_id: {_eq: $bookId}, list: {user_id: {_eq: $userId}}}
    order_by: {id: asc}
    limit: $limit
  ) { id list_id book_id edition_id }
}
";

/// A page of one list's books in the list's order. Unranked lists usually
/// have no positions, so the id keeps the order stable between pages.
pub(crate) const LIST_BOOKS: &str = concat!(
    "query ListBooks($listId: Int!, $limit: Int!, $offset: Int!) {
  list_books(
    where: {list_id: {_eq: $listId}}
    order_by: [{position: asc_nulls_last}, {id: asc}]
    limit: $limit
    offset: $offset
  ) {
    id position edition_id
    book { ...BookSummaryFields }
  }
}
",
    book_summary_fields!()
);

pub(crate) const INSERT_LIST: &str = concat!(
    "mutation InsertList($object: ListInput!) {
  insert_list(object: $object) { errors id list { ...ListFields } }
}
",
    list_fields!()
);

/// `object` is sent whole: the client resends the list as read with the edits.
pub(crate) const UPDATE_LIST: &str = concat!(
    "mutation UpdateList($id: Int!, $object: ListInput!) {
  update_list(id: $id, object: $object) { errors id list { ...ListFields } }
}
",
    list_fields!()
);

pub(crate) const DELETE_LIST: &str = "mutation DeleteList($id: Int!) {
  delete_list(id: $id) { success }
}
";

pub(crate) const INSERT_LIST_BOOK: &str = "mutation InsertListBook($object: ListBookInput!) {
  insert_list_book(object: $object) { id list_book { id list_id book_id edition_id } }
}
";

/// Takes the `list_books` row id, not the book's.
pub(crate) const DELETE_LIST_BOOK: &str = "mutation DeleteListBook($id: Int!) {
  delete_list_book(id: $id) { id list_id }
}
";

/// One entry's review, fetched when its book page opens.
pub(crate) const USER_BOOK_REVIEW: &str = concat!(
    "query UserBookReview($id: Int!) {
  user_books_by_pk(id: $id) { id ...ReviewFields }
}
",
    review_fields!()
);

/// Writes or clears the review. The review arguments are written out, not
/// passed in `$object`, so the scope check sees that this needs `write:reviews`.
/// The status goes too, as with a rating: see `Client::set_review`.
pub(crate) const UPDATE_USER_BOOK_REVIEW: &str = concat!(
    "mutation UpdateUserBookReview(
  $id: Int!, $statusId: Int!, $markdown: String, $spoilers: Boolean!, $reviewedAt: date
) {
  update_user_book(id: $id, object: {
    status_id: $statusId
    review_markdown: $markdown
    review_has_spoilers: $spoilers
    reviewed_at: $reviewedAt
  }) {
    error id
    user_book { ...UserBookFields ...ReviewFields }
  }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!(),
    review_fields!()
);

/// Deletes the review by writing an empty `review_slate` document (see
/// `Client::delete_review`). Like [`UPDATE_USER_BOOK_REVIEW`], it resends the
/// status.
pub(crate) const DELETE_USER_BOOK_REVIEW: &str = concat!(
    "mutation DeleteUserBookReview($id: Int!, $statusId: Int!, $slate: jsonb!) {
  update_user_book(id: $id, object: {
    status_id: $statusId
    review_slate: $slate
    review_has_spoilers: false
  }) {
    error id
    user_book { ...UserBookFields ...ReviewFields }
  }
}
",
    user_book_fields!(),
    book_summary_fields!(),
    edition_fields!(),
    review_fields!()
);

/// The user's goals that are not archived, the latest end date first.
pub(crate) const GOALS: &str = concat!(
    "query Goals($userId: Int!) {
  goals(
    where: {user_id: {_eq: $userId}, archived: {_eq: false}}
    order_by: [{end_date: desc}, {id: desc}]
  ) { ...GoalFields }
}
",
    goal_fields!()
);

/// Re-reads a goal when a mutation's payload left it out.
pub(crate) const GOAL_BY_ID: &str = concat!(
    "query GoalById($id: Int!) {
  goals_by_pk(id: $id) { ...GoalFields }
}
",
    goal_fields!()
);

pub(crate) const INSERT_GOAL: &str = concat!(
    "mutation InsertGoal($object: GoalInput!) {
  insert_goal(object: $object) { errors id goal { ...GoalFields } }
}
",
    goal_fields!()
);

/// `object` is the whole goal: fields left out are not kept.
pub(crate) const UPDATE_GOAL: &str = concat!(
    "mutation UpdateGoal($id: Int!, $object: GoalInput!) {
  update_goal(id: $id, object: $object) { errors id goal { ...GoalFields } }
}
",
    goal_fields!()
);

pub(crate) const DELETE_GOAL: &str = "mutation DeleteGoal($id: Int!) {
  delete_goal(id: $id) { errors id }
}
";

/// Other readers' reviews of one book: rows with a review, the user's own
/// (`$me`) left out.
macro_rules! book_reviews_where {
    () => {
        "{book_id: {_eq: $bookId}, has_review: {_eq: true}, user_id: {_neq: $me}}"
    };
}

/// A page of other readers' reviews of a book, the most liked first, then the
/// newest. Paged by offset; see `Client::book_reviews`.
pub(crate) const BOOK_REVIEWS: &str = concat!(
    "query BookReviews($bookId: Int!, $me: Int!, $limit: Int!, $offset: Int!) {
  user_books(
    where: ",
    book_reviews_where!(),
    "
    order_by: [{likes_count: desc}, {reviewed_at: desc_nulls_last}, {id: desc}]
    limit: $limit
    offset: $offset
  ) {
    id rating likes_count reviewed_at review_has_spoilers review_raw
    user { id username name cached_image }
  }
}
"
);

/// How many reviews [`BOOK_REVIEWS`] pages through.
pub(crate) const BOOK_REVIEWS_COUNT: &str = concat!(
    "query BookReviewsCount($bookId: Int!, $me: Int!) {
  user_books_aggregate(where: ",
    book_reviews_where!(),
    ") { aggregate { count } }
}
"
);

/// A vibe: one of Hardcover's lists of recommended books.
/// `cached_book_ids` holds its books' ids in Hardcover's ranked order.
macro_rules! vibe_fields {
    () => {
        "fragment VibeFields on vibes {
  id title description slug vibe_type books_generated_at cached_book_ids
  featured privacy_setting_id user_id likes_count
  user { username }
}
"
    };
}

/// The user's system vibes: Recommendations (`vibe_type` 1) and Top Picks
/// (3). Every user has them once Hardcover made them
/// ([`ENSURE_SYSTEM_VIBES`]).
pub(crate) const SYSTEM_VIBES: &str = concat!(
    "query SystemVibes($userId: Int!) {
  vibes(
    where: {user_id: {_eq: $userId}, vibe_type: {_in: [1, 3]}}
    order_by: [{vibe_type: asc}, {id: asc}]
  ) { ...VibeFields }
}
",
    vibe_fields!()
);

/// The vibes Hardcover features, the most recently featured first.
pub(crate) const FEATURED_VIBES: &str = concat!(
    "query FeaturedVibes($limit: Int!) {
  vibes(
    where: {featured: {_eq: true}}
    order_by: [{featured_at: desc_nulls_last}, {id: desc}]
    limit: $limit
  ) { ...VibeFields }
}
",
    vibe_fields!()
);

/// One vibe, for its page, and read just before a write to it.
pub(crate) const VIBE_BY_ID: &str = concat!(
    "query VibeById($id: Int!) {
  vibes_by_pk(id: $id) { ...VibeFields }
}
",
    vibe_fields!()
);

/// The user's system vibe ids, making the ones missing.
pub(crate) const ENSURE_SYSTEM_VIBES: &str = "mutation EnsureSystemVibes {
  ensure_system_vibes { ids }
}
";

/// Leaves a book out of a vibe ("Never recommend…"). `object` resends the
/// vibe's title, description and privacy as read just before, with the
/// exclusion: fields left out may be cleared. No query reads exclusions
/// back, so it can't be undone from the app.
pub(crate) const EXCLUDE_FROM_VIBE: &str = concat!(
    "mutation ExcludeFromVibe($id: Int!, $object: VibeInput!) {
  update_vibe(id: $id, object: $object) { id errors vibe { ...VibeFields } }
}
",
    vibe_fields!()
);

/// Every document the client sends, by name. Validated against
/// `schema/hardcover.graphql` in the tests below and against the live schema by
/// the `api_check` example.
pub(crate) const ALL: &[(&str, &str)] = &[
    ("ME", ME),
    ("LIBRARY_PAGE", LIBRARY_PAGE),
    ("USER_BOOK_BY_ID", USER_BOOK_BY_ID),
    ("USER_BOOK_FOR_USER", USER_BOOK_FOR_USER),
    ("USER_BOOK_FOR_BOOK", USER_BOOK_FOR_BOOK),
    ("SEARCH_BOOKS", SEARCH_BOOKS),
    ("SEARCH_AUTHORS", SEARCH_AUTHORS),
    ("SEARCH_SERIES", SEARCH_SERIES),
    ("BOOKS_TRENDING", BOOKS_TRENDING),
    ("BOOK_DETAILS", BOOK_DETAILS),
    ("BOOKS_BY_TAG", BOOKS_BY_TAG),
    ("BOOKS_BY_TAG_WITH", BOOKS_BY_TAG_WITH),
    ("BOOK_TAGS", BOOK_TAGS),
    ("SERIES_PAGE", SERIES_PAGE),
    ("AUTHOR_PAGE", AUTHOR_PAGE),
    ("BOOKS_BY_IDS", BOOKS_BY_IDS),
    ("TRENDING_BOOKS", TRENDING_BOOKS),
    ("PROGRESS_CONTEXT", PROGRESS_CONTEXT),
    ("INSERT_USER_BOOK", INSERT_USER_BOOK),
    ("UPDATE_USER_BOOK", UPDATE_USER_BOOK),
    ("DELETE_USER_BOOK", DELETE_USER_BOOK),
    ("INSERT_USER_BOOK_READ", INSERT_USER_BOOK_READ),
    ("UPDATE_USER_BOOK_READ", UPDATE_USER_BOOK_READ),
    ("DELETE_USER_BOOK_READ", DELETE_USER_BOOK_READ),
    ("BOOK_EDITIONS", BOOK_EDITIONS),
    ("EDITION_OWNED", EDITION_OWNED),
    ("USER_BOOK_REVIEW", USER_BOOK_REVIEW),
    ("UPDATE_USER_BOOK_REVIEW", UPDATE_USER_BOOK_REVIEW),
    ("DELETE_USER_BOOK_REVIEW", DELETE_USER_BOOK_REVIEW),
    ("GOALS", GOALS),
    ("GOAL_BY_ID", GOAL_BY_ID),
    ("INSERT_GOAL", INSERT_GOAL),
    ("UPDATE_GOAL", UPDATE_GOAL),
    ("DELETE_GOAL", DELETE_GOAL),
    ("LISTS", LISTS),
    ("LIST_BY_ID", LIST_BY_ID),
    ("LIST_MEMBERSHIPS", LIST_MEMBERSHIPS),
    ("BOOK_LIST_MEMBERSHIPS", BOOK_LIST_MEMBERSHIPS),
    ("LIST_BOOKS", LIST_BOOKS),
    ("INSERT_LIST", INSERT_LIST),
    ("UPDATE_LIST", UPDATE_LIST),
    ("DELETE_LIST", DELETE_LIST),
    ("INSERT_LIST_BOOK", INSERT_LIST_BOOK),
    ("DELETE_LIST_BOOK", DELETE_LIST_BOOK),
    ("BOOK_REVIEWS", BOOK_REVIEWS),
    ("BOOK_REVIEWS_COUNT", BOOK_REVIEWS_COUNT),
    ("UPCOMING_IN_SERIES", UPCOMING_IN_SERIES),
    ("SERIES_OVERVIEW", SERIES_OVERVIEW),
    ("SYSTEM_VIBES", SYSTEM_VIBES),
    ("FEATURED_VIBES", FEATURED_VIBES),
    ("VIBE_BY_ID", VIBE_BY_ID),
    ("ENSURE_SYSTEM_VIBES", ENSURE_SYSTEM_VIBES),
    ("EXCLUDE_FROM_VIBE", EXCLUDE_FROM_VIBE),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::sdl;

    #[test]
    fn every_document_validates_against_the_schema() {
        let schema = sdl::schema();
        let mut failures = Vec::new();
        for (name, doc) in ALL {
            if let Err(e) = sdl::validate(schema, name, doc) {
                failures.push(format!("{name}:\n{e}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }

    #[test]
    fn validator_rejects_bad_documents() {
        // Guards against the validation test silently passing everything.
        let schema = sdl::schema();
        assert!(sdl::validate(schema, "bad_field", "query { me { id nope } }").is_err());
        assert!(
            sdl::validate(
                schema,
                "bad_type",
                "mutation($o: UserBookInput!) { insert_user_book(object: $o) { id } }"
            )
            .is_err()
        );
        assert!(
            sdl::validate(
                schema,
                "no_such_root",
                "query { user_books_by_slug(id: 1) { id } }"
            )
            .is_err()
        );
    }

    fn committed_scope_map() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../schema/hardcover-capabilities.json"
        ))
        .unwrap()
    }

    fn usages(docs: &[(&'static str, &'static str)]) -> Vec<(&'static str, sdl::Usage)> {
        docs.iter()
            .map(|(name, doc)| (*name, sdl::usage(sdl::schema(), name, doc).unwrap()))
            .collect()
    }

    #[test]
    fn the_requested_scopes_cover_every_document() {
        let report = sdl::check_scopes(
            &committed_scope_map(),
            &crate::auth::DEFAULT_SCOPES,
            &usages(ALL),
        )
        .unwrap();
        assert!(report.problems.is_empty(), "{:#?}", report.problems);
        assert!(report.notes.is_empty(), "{:#?}", report.notes);
    }

    #[test]
    fn scope_check_sees_argument_and_column_gates() {
        // `review_markdown` is gated by write:reviews, which grants no operation of its own.
        const LITERAL_REVIEW: &str = "mutation R($id: Int!) {
  update_user_book(id: $id, object: {review_markdown: \"Great\"}) { id }
}";
        let caps = committed_scope_map();
        let library_only = ["read:me", "read:catalog", "read:library", "write:library"];

        let docs = usages(&[("LITERAL_REVIEW", LITERAL_REVIEW)]);
        let report = sdl::check_scopes(&caps, &library_only, &docs).unwrap();
        assert_eq!(report.problems.len(), 1, "{:#?}", report.problems);
        assert!(report.problems[0].contains("`review_markdown`"));
        let mut with_reviews = library_only.to_vec();
        with_reviews.push("write:reviews");
        let report = sdl::check_scopes(&caps, &with_reviews, &docs).unwrap();
        assert!(report.problems.is_empty(), "{:#?}", report.problems);

        // Through a variable it can only be a note.
        let docs = usages(&[("UPDATE_USER_BOOK", UPDATE_USER_BOOK)]);
        let report = sdl::check_scopes(&caps, &library_only, &docs).unwrap();
        assert!(report.problems.is_empty(), "{:#?}", report.problems);
        assert!(
            report.notes.iter().any(|n| n.contains("`review_markdown`")),
            "{:#?}",
            report.notes
        );

        // Written out with variable values, the review arguments are still seen.
        let docs = usages(&[("UPDATE_USER_BOOK_REVIEW", UPDATE_USER_BOOK_REVIEW)]);
        let report = sdl::check_scopes(&caps, &library_only, &docs).unwrap();
        for arg in ["review_markdown", "review_has_spoilers", "reviewed_at"] {
            assert!(
                report
                    .problems
                    .iter()
                    .any(|p| p.contains(&format!("`{arg}`"))),
                "{arg}: {:#?}",
                report.problems
            );
        }
        let docs = usages(&[("DELETE_USER_BOOK_REVIEW", DELETE_USER_BOOK_REVIEW)]);
        let report = sdl::check_scopes(&caps, &library_only, &docs).unwrap();
        for arg in ["review_slate", "review_has_spoilers"] {
            assert!(
                report
                    .problems
                    .iter()
                    .any(|p| p.contains(&format!("`{arg}`"))),
                "{arg}: {:#?}",
                report.problems
            );
        }

        // `email` is a column gated by read:me:email.
        let docs = usages(&[("EMAIL", "query E { me { id email } }")]);
        let report = sdl::check_scopes(&caps, &["read:me:content"], &docs).unwrap();
        assert!(
            report.problems.iter().any(|p| p.contains("`email`")),
            "{:#?}",
            report.problems
        );

        // A scope that disappeared from the map breaks sign-in.
        let report = sdl::check_scopes(&caps, &["read:me", "read:nothing"], &[]).unwrap();
        assert_eq!(report.problems.len(), 1);
    }

    #[test]
    fn every_document_has_one_top_level_field() {
        use apollo_compiler::ExecutableDocument;
        let schema = sdl::schema();
        for (name, doc) in ALL {
            let doc = ExecutableDocument::parse_and_validate(schema, *doc, *name).unwrap();
            let op = doc.operations.iter().next().unwrap();
            assert_eq!(
                op.selection_set.selections.len(),
                1,
                "{name} has more than one top-level field"
            );
        }
    }
}

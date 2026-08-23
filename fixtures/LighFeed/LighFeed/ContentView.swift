import SwiftUI

struct Post: Identifiable {
    let id: Int
    var title: String { "Post \(id)" }
}

struct ContentView: View {
    private let posts = (1...30).map { Post(id: $0) }
    @State private var selected: Post?

    var body: some View {
        NavigationStack {
            if let post = selected {
                VStack(spacing: 16) {
                    Text("PostDetail")
                        .font(.title)
                        .accessibilityIdentifier("PostDetail")
                    Text(post.title)
                        .accessibilityIdentifier("PostDetailTitle")
                    Button("BackToFeed") {
                        selected = nil
                    }
                    .accessibilityIdentifier("BackToFeed")
                }
                .navigationTitle("Detail")
            } else {
                List(posts) { post in
                    Button(post.title) {
                        selected = post
                    }
                    .accessibilityIdentifier("post-\(post.id)")
                }
                .accessibilityIdentifier("FeedList")
                .navigationTitle("Feed")
            }
        }
    }
}

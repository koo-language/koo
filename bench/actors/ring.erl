%% A thousand gen_servers in a ring pass a message round, twenty million times in all.
-module(ring).
-behaviour(gen_server).
-export([main/0, init/1, handle_call/3, handle_cast/2]).

init(Main) -> {ok, {none, Main}}.
handle_call({link, To}, _From, {_, Main}) -> {reply, ok, {To, Main}}.
handle_cast({pass, 0, Passed}, {Next, Main}) ->
    Main ! {done, Passed + 1},
    {noreply, {Next, Main}};
handle_cast({pass, Left, Passed}, {Next, Main}) ->
    gen_server:cast(Next, {pass, Left - 1, Passed + 1}),
    {noreply, {Next, Main}}.

main() ->
    Nodes = [element(2, gen_server:start(?MODULE, self(), [])) || _ <- lists:seq(1, 1000)],
    lists:foreach(
        fun({Node, Next}) -> gen_server:call(Node, {link, Next}) end,
        lists:zip(Nodes, tl(Nodes) ++ [hd(Nodes)])),
    gen_server:cast(hd(Nodes), {pass, 20000000, 0}),
    receive {done, Total} -> io:format("~p~n", [Total]) end,
    halt().

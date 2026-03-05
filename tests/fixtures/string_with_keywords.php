<?php
namespace Foo;

class StringEdgeCase
{
    public function __construct(
        \Foo\Service $service
    ) {
        $x = 'namespace Foo; class Fake {}';
        $y = "interface FakeInterface {}";
    }
}

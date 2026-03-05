<?php
namespace Magento\Framework;

// This is a class comment with the class keyword
/* Another class reference */

class CommentTest
{
    /** @var string This has class keyword in docblock */
    private string $name;

    public function __construct(string $name = 'class_default')
    {
        // $this->classMethod();
        $this->name = $name;
    }
}

<?php
namespace Magento\Framework\Data;

interface CollectionInterface
    extends \Countable,
            \IteratorAggregate,
            \Magento\Framework\Data\SearchResultInterface
{
    public function getItems(): array;
    public function getTotalCount(): int;
}
